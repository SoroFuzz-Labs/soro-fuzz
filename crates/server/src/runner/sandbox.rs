//! How a fuzz subprocess is isolated and executed. `Sandbox` is the
//! extension point — `SubprocessRunner` never spawns a process directly, it
//! always goes through one of these, so swapping isolation strategies never
//! touches the runner's output-parsing logic.
//!
//! contributors: add a new strategy (nsjail, firejail, a remote executor,
//! ...) by implementing this trait and adding a matching `SandboxKind`
//! variant in `config.rs`.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Clone, Default)]
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    /// `None` if the process never exited on its own (killed after
    /// `timed_out`, or the platform couldn't report a code).
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to spawn subprocess: {0}")]
    Spawn(#[from] std::io::Error),
}

/// contributors: `run` intentionally never returns `Err` for a nonzero exit
/// or a timeout — those are ordinary fuzzing outcomes the caller inspects
/// via `SandboxOutput` (a crash *is* a successful run, from the sandbox's
/// point of view). `Err` is reserved for the sandbox failing to even start
/// the command.
#[async_trait]
pub trait Sandbox: Send + Sync {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<SandboxOutput, SandboxError>;
}

/// Runs the command directly as a child of the server process. Only safe on
/// a dev box or CI runner that already has the toolchain installed and
/// nothing else worth protecting from a misbehaving fuzz target — this is
/// what `FUZZ_SANDBOX=local` opts into.
pub struct LocalSandbox;

#[async_trait]
impl Sandbox for LocalSandbox {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<SandboxOutput, SandboxError> {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd);
        run_with_timeout(command, timeout).await
    }
}

/// Runs the command inside an already-running sidecar container via `docker
/// exec` — what docker-compose's `toolchain` service is for. `cwd` must be a
/// path *inside that container* (see `Config::fuzz_workspace_dir`'s doc
/// comment); this sandbox doesn't start or stop the container itself, only
/// `exec`s into it, so `MAX_CONCURRENT_CAMPAIGNS` concurrent jobs share one
/// container's resource limits (set on the container at `docker run`/
/// compose time, not per-exec).
pub struct DockerExecSandbox {
    pub container: String,
}

#[async_trait]
impl Sandbox for DockerExecSandbox {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<SandboxOutput, SandboxError> {
        let mut command = Command::new("docker");
        command
            .arg("exec")
            .arg("-w")
            .arg(cwd.display().to_string())
            .arg(&self.container)
            .arg(program)
            .args(args);
        run_with_timeout(command, timeout).await
    }
}

/// After a timeout kill, how long we still wait for the output pipes to reach
/// EOF before abandoning them. Long enough to capture a normally-terminating
/// child's final bytes, short enough that a process which inherited the pipe
/// and outlived its parent cannot stall the caller.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// SIGKILLs the entire process group led by `child`.
///
/// The child is spawned into its own process group (`process_group(0)` below),
/// so its pid doubles as the group id and signalling the *negative* pid
/// reaches the child and every process it spawned. That is what lets a timeout
/// reap a shell's grandchildren — e.g. the `sleep`/`ping` behind a
/// `sh -c`/`cmd /C` shim — instead of orphaning one that still holds our
/// stdout/stderr pipe open.
#[cfg(unix)]
async fn terminate_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: `kill(2)` with a group target touches no memory we own; a
        // pid whose group has already exited yields `ESRCH`, which we drop.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

/// Windows has no process group we can signal the way Unix does, so we ask
/// `taskkill` to walk and kill the child's whole tree by pid (`/T`). We wait
/// for it to finish *before* the caller reaps the direct child: killing the
/// child first would orphan its grandchildren and break the parent links `/T`
/// follows, leaving a grandchild holding our pipe open (and, on Windows, a
/// blocking pipe read the runtime then joins at shutdown).
#[cfg(windows)]
async fn terminate_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[cfg(not(any(unix, windows)))]
async fn terminate_group(_child: &tokio::process::Child) {}

async fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<SandboxOutput, SandboxError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Put the child in its own process group so a timeout can SIGKILL the whole
    // group (the child *and* anything it spawned), not just the process we hold
    // a handle to. Otherwise a grandchild that inherited the output pipe keeps
    // its write end open after we kill the parent, and the drain below blocks
    // until that grandchild exits on its own — the exact hang this timeout
    // exists to break. (Windows can't set this at spawn; `terminate_group`
    // kills the tree by pid via taskkill instead.)
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn()?;

    // Drained concurrently with `wait`, not after it: a chatty subprocess
    // can otherwise deadlock by filling a pipe buffer while we're blocked
    // waiting for it to exit.
    let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
    let stdout_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stdout_pipe.read_to_string(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf).await;
        buf
    });
    // Kept so a timed-out drain that a survivor is still holding open can be
    // cancelled rather than left to block; unused on the normal-exit path.
    let stdout_abort = stdout_task.abort_handle();
    let stderr_abort = stderr_task.abort_handle();

    let (timed_out, exit_code) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => (false, status.code()),
        Ok(Err(_)) => (false, None),
        Err(_) => {
            // Deadline hit. Kill the whole tree (see `terminate_group`) so any
            // grandchildren the target spawned die too — awaited before we reap
            // the direct child, so the tree is still walkable — then reap it.
            terminate_group(&child).await;
            let _ = child.kill().await;
            (true, None)
        }
    };

    // Collect whatever the pipes captured. On the normal-exit path they reach
    // EOF the moment the child (and, on Unix, its group) exits, so this returns
    // at once and we wait however long the last bytes take. After a timeout we
    // bound the wait: output already captured is kept, but a pipe still held
    // open by a survivor past `DRAIN_GRACE` is abandoned rather than allowed to
    // block the caller indefinitely.
    let (stdout, stderr) = if timed_out {
        let drain = async {
            (
                stdout_task.await.unwrap_or_default(),
                stderr_task.await.unwrap_or_default(),
            )
        };
        match tokio::time::timeout(DRAIN_GRACE, drain).await {
            Ok(pair) => pair,
            Err(_) => {
                stdout_abort.abort();
                stderr_abort.abort();
                (String::new(), String::new())
            }
        }
    } else {
        (
            stdout_task.await.unwrap_or_default(),
            stderr_task.await.unwrap_or_default(),
        )
    };

    Ok(SandboxOutput {
        stdout,
        stderr,
        exit_code,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Portable across the Windows dev box this was written on and the Linux
    // CI/toolchain image it'll actually run cargo-fuzz on — neither `cmd`
    // nor `sh` is assumed available on the other platform.
    fn echo_command(text: &str) -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), format!("echo {text}")])
        } else {
            ("sh", vec!["-c".to_string(), format!("echo {text}")])
        }
    }

    fn exit_command(code: i32) -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), format!("exit {code}")])
        } else {
            ("sh", vec!["-c".to_string(), format!("exit {code}")])
        }
    }

    // `timeout.exe` refuses redirected stdin ("Input redirection is not
    // supported") which is exactly what `run_with_timeout` sets up, so a
    // few pings to loopback stands in for a portable sleep on Windows.
    fn sleep_command(secs: u32) -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd",
                vec![
                    "/C".to_string(),
                    format!("ping -n {} 127.0.0.1 >NUL", secs + 1),
                ],
            )
        } else {
            ("sh", vec!["-c".to_string(), format!("sleep {secs}")])
        }
    }

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let (program, args) = echo_command("hello-sandbox");
        let output = LocalSandbox
            .run(
                program,
                &args,
                &std::env::temp_dir(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        assert!(
            output.stdout.contains("hello-sandbox"),
            "stdout was: {:?}",
            output.stdout
        );
        assert_eq!(output.exit_code, Some(0));
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn reports_nonzero_exit_code() {
        let (program, args) = exit_command(7);
        let output = LocalSandbox
            .run(
                program,
                &args,
                &std::env::temp_dir(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        assert_eq!(output.exit_code, Some(7));
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn kills_and_flags_a_command_that_outlives_the_timeout() {
        let (program, args) = sleep_command(30);
        let started = std::time::Instant::now();

        let output = LocalSandbox
            .run(
                program,
                &args,
                &std::env::temp_dir(),
                Duration::from_millis(200),
            )
            .await
            .unwrap();

        assert!(output.timed_out);
        assert_eq!(output.exit_code, None);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the 30s sleep should have been killed almost immediately, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn spawn_failure_of_a_nonexistent_program_is_an_error() {
        let result = LocalSandbox
            .run(
                "this-program-does-not-exist-anywhere",
                &[],
                &std::env::temp_dir(),
                Duration::from_secs(5),
            )
            .await;
        assert!(result.is_err());
    }
}
