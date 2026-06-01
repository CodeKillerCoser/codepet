#!/usr/bin/env node
import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const COLLECTOR_URL = process.env.CODE_PET_COLLECTOR_URL || "http://127.0.0.1:47621/hook";
const APPROVAL_WAIT_MS = Number(process.env.CODE_PET_APPROVAL_WAIT_MS || 590000);

function argValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString("utf8");
}

function parsePayload(stdinText, eventName) {
  if (stdinText.trim()) {
    try {
      const parsed = JSON.parse(stdinText);
      if (eventName && !parsed.hook_event_name) {
        parsed.hook_event_name = eventName;
      }
      return parsed;
    } catch {
      return {
        hook_event_name: eventName || "Message",
        message: stdinText.trim(),
      };
    }
  }

  return {
    hook_event_name: eventName || "Notification",
    source: "code-pet-hook",
    message: process.argv.slice(2).join(" "),
    cwd: process.cwd(),
  };
}

function sourceContext() {
  return {
    pid: process.pid,
    ppid: process.ppid,
    terminalProgram: process.env.TERM_PROGRAM || null,
    termSessionId: process.env.TERM_SESSION_ID || null,
    ttyPath: controllingTty(),
    tmuxPane: process.env.TMUX_PANE || null,
    weztermPane: process.env.WEZTERM_PANE || process.env.WEZTERM_PANE_ID || null,
    kittyWindowId: process.env.KITTY_WINDOW_ID || null,
    appBundleId: process.env.__CFBundleIdentifier || null,
  };
}

function controllingTty() {
  try {
    const result = spawnSync("sh", ["-c", "tty < /dev/tty"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 500,
    });
    const tty = result.stdout?.trim();
    return tty && tty !== "not a tty" ? tty : null;
  } catch {
    return null;
  }
}

function withSourceContext(payload) {
  return {
    ...payload,
    cwd: payload.cwd || process.cwd(),
    code_pet: {
      ...sourceContext(),
      ...(payload.code_pet || payload.codePet || {}),
    },
  };
}

function spoolEvent(body) {
  const spoolPath = join(homedir(), ".code-pet", "spool", "events.jsonl");
  mkdirSync(dirname(spoolPath), { recursive: true });
  appendFileSync(spoolPath, `${JSON.stringify({ ...body, spooledAt: new Date().toISOString() })}\n`);
}

async function postEvent(body) {
  try {
    const response = await fetch(COLLECTOR_URL, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      spoolEvent(body);
      return null;
    }
    return await response.json();
  } catch {
    spoolEvent(body);
    return null;
  }
}

async function waitForApproval(eventId) {
  const waitUrl = COLLECTOR_URL.replace(/\/hook$/, `/approvals/${encodeURIComponent(eventId)}/wait?timeoutMs=${Math.max(1, APPROVAL_WAIT_MS)}`);
  try {
    const response = await fetch(waitUrl);
    if (!response.ok) {
      return null;
    }
    const body = await response.json();
    return body.decision || null;
  } catch {
    return null;
  }
}

function hookDecisionOutput(eventName, decision) {
  const behavior = decision?.behavior === "deny" ? "deny" : "allow";
  const hookDecision = { behavior };
  if (behavior === "deny" && decision?.message) {
    hookDecision.message = decision.message;
  }
  return {
    hookSpecificOutput: {
      hookEventName: eventName,
      decision: hookDecision,
    },
  };
}

function cursorAllowOutput() {
  return {
    continue: true,
    permission: "allow",
  };
}

function forwardToPrevious(stdinText) {
  const encoded = argValue("--forward") || forwardBase64Arg();
  if (!encoded) {
    return;
  }

  try {
    const command = JSON.parse(encoded);
    if (!Array.isArray(command) || command.length === 0) {
      return;
    }
    spawnSync(command[0], command.slice(1), {
      input: stdinText,
      stdio: ["pipe", "ignore", "ignore"],
      shell: false,
      timeout: 5000,
    });
  } catch {
    // Hook forwarding must never break the agent that invoked the hook.
  }
}

function forwardBase64Arg() {
  const encoded = argValue("--forward-b64");
  if (!encoded) {
    return undefined;
  }
  try {
    return Buffer.from(encoded, "base64").toString("utf8");
  } catch {
    return undefined;
  }
}

const stdinText = await readStdin();
const agent = process.env.CODE_PET_AGENT || argValue("--agent") || "unknown";
const eventName = argValue("--event") || process.env.CODE_PET_EVENT;
const payload = withSourceContext(parsePayload(stdinText, eventName));
const event = await postEvent({ agent, payload });
forwardToPrevious(stdinText);
if (payload.hook_event_name === "PermissionRequest" && event?.id) {
  const decision = await waitForApproval(event.id);
  if (decision) {
    process.stdout.write(`${JSON.stringify(hookDecisionOutput(payload.hook_event_name, decision))}\n`);
  }
} else if (agent === "cursor") {
  process.stdout.write(`${JSON.stringify(cursorAllowOutput())}\n`);
}
