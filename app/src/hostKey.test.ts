// Run with: node --test app/src/hostKey.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { findHostKeyChange, findKeyPermissionProblem, isProgramPathTitle } from "./hostKey.ts";

// What OpenSSH prints when a saved server was reinstalled (address made up).
const WARNING = [
  "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
  "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
  "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
  "IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!",
  "The fingerprint for the ED25519 key sent by the remote host is",
  "SHA256:KWu/ZhGMtttMi2VGgFv5/DjhBCw0QKbMyBtbvi+t2kQ.",
  "Offending ECDSA key in /Users/someone/.ssh/known_hosts:138",
  "Host key for 203.0.113.7 has changed and you have requested strict checking.",
  "Host key verification failed.",
].join("\r\n");

test("finds the server and the new fingerprint", () => {
  assert.deepEqual(findHostKeyChange(WARNING, 255), {
    reported: "203.0.113.7",
    fingerprint: "SHA256:KWu/ZhGMtttMi2VGgFv5/DjhBCw0QKbMyBtbvi+t2kQ",
  });
});

test("a non-standard port comes through in ssh's own [host]:port form", () => {
  const w = WARNING.replace("203.0.113.7 has", "[s1.example.com]:2279 has");
  assert.equal(findHostKeyChange(w, 255)?.reported, "[s1.example.com]:2279");
});

test("survives Windows ConPTY colour and title codes", () => {
  const w = "\x1b]0;C:\\ssh.exe\x07\x1b[?25l" + WARNING.replace(/\r\n/g, "\x1b[K\r\n");
  assert.equal(findHostKeyChange(w, 255)?.reported, "203.0.113.7");
});

test("ordinary output, or only half of the warning, is not a host key change", () => {
  assert.equal(findHostKeyChange("Welcome to Ubuntu\r\nroot@web:~# ", 255), null);
  assert.equal(findHostKeyChange("echo REMOTE HOST IDENTIFICATION HAS CHANGED", 255), null);
  assert.equal(findHostKeyChange("Host key for x has changed", 255), null);
});

test("only when the session ENDS on ssh's refusal, with ssh's exit code", () => {
  assert.equal(findHostKeyChange(WARNING, 0), null, "a logged-in session exiting 0");
  assert.equal(findHostKeyChange(WARNING, null), null);
  assert.equal(
    findHostKeyChange(WARNING + "\r\nroot@web:~# exit\r\n", 255),
    null,
    "printed during a session that then went on"
  );
});

test("the real warning captured from OpenSSH is recognised", () => {
  // Captured 2026-10-23 from a throwaway local sshd whose key was swapped.
  const real =
    "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n" +
    "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n" +
    "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n" +
    "IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\n" +
    "The fingerprint for the ED25519 key sent by the remote host is\n" +
    "SHA256:Em06bjQBmJ7VxyK0PBYf3FdegP9RkhJpi35vogx8Svg.\n" +
    "Offending ED25519 key in /tmp/kh:1\n" +
    "Host key for [127.0.0.1]:42722 has changed and you have requested strict checking.\n" +
    "Host key verification failed.\n";
  assert.deepEqual(findHostKeyChange(real, 255), {
    reported: "[127.0.0.1]:42722",
    fingerprint: "SHA256:Em06bjQBmJ7VxyK0PBYf3FdegP9RkhJpi35vogx8Svg",
  });
});

test("a key file other accounts can read is recognised, Windows or Mac", () => {
  // As Windows OpenSSH printed it for a user (2026-10-24).
  const win =
    "Bad permissions. Try removing permissions for user: NT AUTHORITY\\\\Authenticated Users (S-1-5-11) on file F:/Office-PC.\r\n" +
    "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n" +
    "@         WARNING: UNPROTECTED PRIVATE KEY FILE!          @\r\n" +
    "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n" +
    "Permissions for 'F:/Office-PC' are too open.\r\n" +
    "This private key will be ignored.\r\n" +
    'Load key "C:\\\\Users\\\\Jhinuk/Downloads/Office-PC": bad permissions\r\n' +
    "root@203.0.113.7: Permission denied (publickey,gssapi-keyex,gssapi-with-mic).\r\n";
  assert.deepEqual(findKeyPermissionProblem(win, 255), { path: "C:\\Users\\Jhinuk/Downloads/Office-PC" });
  const mac = 'WARNING: UNPROTECTED PRIVATE KEY FILE!\nLoad key "/Users/x/.ssh/id_ed25519": bad permissions\n';
  assert.equal(findKeyPermissionProblem(mac, 255)?.path, "/Users/x/.ssh/id_ed25519");
  assert.equal(findKeyPermissionProblem(mac, 0), null, "a session that logged in anyway");
  assert.equal(findKeyPermissionProblem("Permission denied (publickey).", 255), null);
});

test("a program path is not a useful tab title; a shell's title is", () => {
  assert.ok(isProgramPathTitle("C:\\WINDOWS\\System32\\OpenSSH\\ssh.EXE"));
  assert.ok(isProgramPathTitle("C:/Windows/System32/cmd.exe"));
  assert.ok(!isProgramPathTitle("/usr/bin/tools"), "a real directory keeps its title");
  assert.ok(!isProgramPathTitle("D:\\builds\\setup.exe"), "only console programs are dropped");
  assert.ok(!isProgramPathTitle("root@web: ~"));
  assert.ok(!isProgramPathTitle("root@web: /var/www"));
  assert.ok(!isProgramPathTitle("Contabo"));
});
