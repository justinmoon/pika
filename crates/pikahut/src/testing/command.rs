use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use super::TestContext;

/// Typed command execution specification for integration tests.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pikahut::testing::CommandSpec;
///
/// let spec = CommandSpec::cargo()
///     .arg("--version")
///     .timeout(Duration::from_secs(10))
///     .retries(1)
///     .capture_name("cargo-version");
///
/// assert_eq!(spec.program(), "cargo");
/// ```
#[derive(Debug, Clone)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    timeout: Option<Duration>,
    retries: u32,
    capture_name: Option<String>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            timeout: None,
            retries: 0,
            capture_name: None,
        }
    }

    /// Helper for `cargo` invocations.
    pub fn cargo() -> Self {
        Self::new("cargo").capture_name("cargo")
    }

    /// Helper for `xcodebuild` invocations.
    pub fn xcodebuild() -> Self {
        Self::new("xcodebuild").capture_name("xcodebuild")
    }

    /// Helper for Gradle wrapper invocations.
    pub fn gradlew() -> Self {
        Self::new("./gradlew").capture_name("gradlew")
    }

    /// Helper for `node` invocations.
    pub fn node() -> Self {
        Self::new("node").capture_name("node")
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in envs {
            self.env.insert(key.into(), value.into());
        }
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    pub fn capture_name(mut self, capture_name: impl Into<String>) -> Self {
        self.capture_name = Some(capture_name.into());
        self
    }

    fn capture_base(&self) -> String {
        self.capture_name
            .as_deref()
            .map(sanitize_capture_name)
            .unwrap_or_else(|| sanitize_capture_name(&self.program))
    }
}

/// Captured output and artifact paths for a command execution.
#[derive(Debug)]
pub struct CommandOutput {
    pub status: ExitStatus,
    pub attempts: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// Long-running spawned command handle. Stdout/stderr are captured directly
/// into artifact files under the parent [`TestContext`].
#[derive(Debug)]
pub struct SpawnHandle {
    child: Option<Child>,
    command: String,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

impl SpawnHandle {
    pub fn wait(mut self) -> Result<ExitStatus> {
        let Some(mut child) = self.child.take() else {
            bail!("spawn handle already consumed for `{}`", self.command);
        };
        child
            .wait()
            .with_context(|| format!("wait for `{}`", self.command))
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        child
            .try_wait()
            .with_context(|| format!("poll `{}`", self.command))
    }

    pub fn kill(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        child
            .kill()
            .with_context(|| format!("kill `{}`", self.command))
    }
}

impl Drop for SpawnHandle {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        if child.try_wait().ok().flatten().is_some() {
            return;
        }

        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Command executor bound to a test context for artifact capture.
#[derive(Debug)]
pub struct CommandRunner<'ctx> {
    context: &'ctx TestContext,
}

impl<'ctx> CommandRunner<'ctx> {
    pub fn new(context: &'ctx TestContext) -> Self {
        Self { context }
    }

    pub fn run(&self, spec: &CommandSpec) -> Result<CommandOutput> {
        let command_artifacts = self.context.ensure_artifact_subdir("commands")?;
        let max_attempts = spec.retries + 1;

        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 1..=max_attempts {
            let output = match self
                .run_once(spec)
                .with_context(|| format!("run command (attempt {attempt}/{max_attempts})"))
            {
                Ok(output) => output,
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            };

            let capture_base = spec.capture_base();
            let stdout_path =
                command_artifacts.join(format!("{capture_base}-attempt-{attempt:02}.stdout.log"));
            let stderr_path =
                command_artifacts.join(format!("{capture_base}-attempt-{attempt:02}.stderr.log"));

            std::fs::write(&stdout_path, &output.stdout)
                .with_context(|| format!("write {}", stdout_path.display()))?;
            std::fs::write(&stderr_path, &output.stderr)
                .with_context(|| format!("write {}", stderr_path.display()))?;

            if output.status.success() {
                return Ok(CommandOutput {
                    status: output.status,
                    attempts: attempt,
                    stdout: output.stdout,
                    stderr: output.stderr,
                    stdout_path,
                    stderr_path,
                });
            }

            last_error = Some(anyhow!(
                "command failed with status {}: {}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                describe_command(spec),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Err(last_error.unwrap_or_else(|| anyhow!("command failed without output")))
    }

    pub fn spawn(&self, spec: &CommandSpec) -> Result<SpawnHandle> {
        let command_artifacts = self.context.ensure_artifact_subdir("commands")?;
        let capture_base = spec.capture_base();
        let stdout_path = command_artifacts.join(format!("{capture_base}-spawn.stdout.log"));
        let stderr_path = command_artifacts.join(format!("{capture_base}-spawn.stderr.log"));

        let stdout_file = File::create(&stdout_path)
            .with_context(|| format!("create {}", stdout_path.display()))?;
        let stderr_file = File::create(&stderr_path)
            .with_context(|| format!("create {}", stderr_path.display()))?;

        let mut command = self.build_command(spec);
        command
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));

        let command_str = describe_command(spec);
        let child = command
            .spawn()
            .with_context(|| format!("spawn `{command_str}`"))?;

        Ok(SpawnHandle {
            child: Some(child),
            command: command_str,
            stdout_path,
            stderr_path,
        })
    }

    fn run_once(&self, spec: &CommandSpec) -> Result<Output> {
        let mut command = self.build_command(spec);

        if let Some(timeout) = spec.timeout {
            run_with_timeout(command, timeout)
        } else {
            command
                .output()
                .with_context(|| format!("spawn `{}`", describe_command(spec)))
        }
    }

    fn build_command(&self, spec: &CommandSpec) -> Command {
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);

        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        } else {
            command.current_dir(self.context.workspace_root());
        }

        if !spec.env.is_empty() {
            command.envs(&spec.env);
        }

        command
    }
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command.spawn().context("spawn timed command")?;
    let start = Instant::now();

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().context("read command output");
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .context("read timed-out command output")?;
            bail!(
                "command timed out after {:.1}s\nstdout:\n{}\nstderr:\n{}",
                timeout.as_secs_f64(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn describe_command(spec: &CommandSpec) -> String {
    let mut parts = Vec::with_capacity(1 + spec.args.len());
    parts.push(shell_quote(&spec.program));
    parts.extend(spec.args.iter().map(|arg| shell_quote(arg)));
    parts.join(" ")
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | ':' | '.' | '='))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn sanitize_capture_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "command".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_python3() -> bool {
        Command::new("python3")
            .arg("--version")
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn run_captures_output_to_memory_and_files() {
        if !has_python3() {
            return;
        }

        let mut context = super::TestContext::builder("command-run")
            .artifact_policy(super::super::ArtifactPolicy::DeleteOnSuccess)
            .build()
            .unwrap();
        let runner = CommandRunner::new(&context);

        let spec = CommandSpec::new("python3")
            .args(["-c", "print('hello-from-command-runner')"])
            .capture_name("python-hello");

        let output = runner.run(&spec).unwrap();
        assert!(output.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello-from-command-runner"));
        assert!(output.stdout_path.exists());
        assert!(output.stderr_path.exists());

        context.mark_success();
    }

    #[test]
    fn retry_policy_retries_until_success() {
        if !has_python3() {
            return;
        }

        let marker_dir = tempfile::tempdir().unwrap();
        let marker_path = marker_dir.path().join("retry-marker");

        let mut context = super::TestContext::builder("command-retry")
            .artifact_policy(super::super::ArtifactPolicy::DeleteOnSuccess)
            .build()
            .unwrap();
        let runner = CommandRunner::new(&context);

        let script = r#"
import pathlib
import sys
marker = pathlib.Path(sys.argv[1])
if marker.exists():
    print('retry-success')
    raise SystemExit(0)
marker.write_text('1', encoding='utf-8')
print('retry-fail', file=sys.stderr)
raise SystemExit(1)
"#;

        let spec = CommandSpec::new("python3")
            .args(["-c", script, marker_path.to_str().unwrap()])
            .retries(1)
            .capture_name("python-retry");

        let output = runner.run(&spec).unwrap();
        assert!(output.success());
        assert_eq!(output.attempts, 2);

        context.mark_success();
    }

    #[test]
    fn timeout_returns_rich_error() {
        if !has_python3() {
            return;
        }

        let mut context = super::TestContext::builder("command-timeout")
            .artifact_policy(super::super::ArtifactPolicy::DeleteOnSuccess)
            .build()
            .unwrap();
        let runner = CommandRunner::new(&context);

        let spec = CommandSpec::new("python3")
            .args(["-c", "import time; time.sleep(1.5)"])
            .timeout(Duration::from_millis(100))
            .capture_name("python-timeout");

        let err = runner.run(&spec).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("timed out"), "{message}");

        context.mark_success();
    }

    #[test]
    fn spawn_streams_to_artifact_files() {
        if !has_python3() {
            return;
        }

        let mut context = super::TestContext::builder("command-spawn")
            .artifact_policy(super::super::ArtifactPolicy::DeleteOnSuccess)
            .build()
            .unwrap();
        let runner = CommandRunner::new(&context);

        let spec = CommandSpec::new("python3")
            .args(["-c", "import time; print('spawn-ok'); time.sleep(0.1)"])
            .capture_name("python-spawn");

        let handle = runner.spawn(&spec).unwrap();
        let stdout_path = handle.stdout_path.clone();
        let status = handle.wait().unwrap();
        assert!(status.success());

        let stdout = std::fs::read_to_string(stdout_path).unwrap();
        assert!(stdout.contains("spawn-ok"));

        context.mark_success();
    }
}
