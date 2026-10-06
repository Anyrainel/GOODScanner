#![cfg(windows)]

use std::{
    os::windows::process::CommandExt,
    process::Command,
    time::{Duration, Instant},
};

use good_tools_app::gui::restart::{exit_for_update, wait_for_parent_exit};

#[test]
fn replacement_waits_until_parent_exits() {
    // This is our own short-lived probe, never the user's running scanner.
    let mut parent = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Milliseconds 400"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
        .unwrap();
    let start = Instant::now();
    wait_for_parent_exit(parent.id(), Duration::from_secs(10)).unwrap();
    assert!(start.elapsed() >= Duration::from_millis(300));
    assert!(parent.try_wait().unwrap().is_some());
}

#[test]
fn live_parent_timeout_refuses_to_start_replacement() {
    let error = wait_for_parent_exit(std::process::id(), Duration::from_millis(20)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn parent_that_already_exited_does_not_block_replacement() {
    let mut parent = Command::new("cmd.exe")
        .args(["/c", "exit", "0"])
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let pid = parent.id();
    parent.wait().unwrap();
    drop(parent);
    wait_for_parent_exit(pid, Duration::from_millis(20)).unwrap();
}

#[test]
#[ignore = "subprocess probe invoked by update_exit_releases_process"]
fn update_exit_probe() {
    // Native libraries can leave background threads behind after their users
    // finish; update exit must release the process regardless.
    std::thread::spawn(|| std::thread::sleep(Duration::from_secs(60)));
    exit_for_update();
}

#[test]
fn update_exit_releases_process() {
    let mut probe = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "update_exit_probe", "--ignored"])
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    wait_for_parent_exit(probe.id(), Duration::from_secs(10)).unwrap();
    assert!(probe.wait().unwrap().success());
}
