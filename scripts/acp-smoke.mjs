// Raw ACP smoke test: verifies the handshake Klets' Rust client performs.
// Usage: node scripts/acp-smoke.mjs "opencode" "acp"
import { spawn } from "node:child_process";

const [command, ...args] = process.argv.slice(2);
if (!command) {
	console.error('usage: node scripts/acp-smoke.mjs <command> [args...]');
	process.exit(1);
}

const child = spawn(command, args, {
	stdio: ["pipe", "pipe", "pipe"],
	shell: process.platform === "win32",
});

let nextId = 1;
const pending = new Map();
let buffer = "";

child.stdout.on("data", (data) => {
	buffer += data.toString();
	let index;
	while ((index = buffer.indexOf("\n")) >= 0) {
		const line = buffer.slice(0, index).trim();
		buffer = buffer.slice(index + 1);
		if (!line) continue;
		let message;
		try {
			message = JSON.parse(line);
		} catch {
			console.log("[non-json]", line.slice(0, 200));
			continue;
		}
		if (message.id !== undefined && pending.has(message.id)) {
			pending.get(message.id)(message);
			pending.delete(message.id);
		} else if (message.method) {
			console.log(`[<- ${message.method}]`, JSON.stringify(message.params).slice(0, 220));
			// Answer client-side requests so the agent isn't left hanging.
			if (message.id !== undefined) {
				send({ jsonrpc: "2.0", id: message.id, result: { outcome: { outcome: "cancelled" } } });
			}
		}
	}
});

child.stderr.on("data", (d) => console.log("[stderr]", d.toString().trim().slice(0, 300)));
child.on("exit", (code) => console.log("[exit]", code));

function send(message) {
	child.stdin.write(`${JSON.stringify(message)}\n`);
}

function request(method, params) {
	const id = nextId++;
	return new Promise((resolve, reject) => {
		const timer = setTimeout(() => reject(new Error(`${method} timed out`)), 120000);
		pending.set(id, (message) => {
			clearTimeout(timer);
			if (message.error) reject(new Error(`${method}: ${JSON.stringify(message.error)}`));
			else resolve(message.result);
		});
		send({ jsonrpc: "2.0", id, method, params });
	});
}

const short = (v) => JSON.stringify(v).slice(0, 400);

try {
	const init = await request("initialize", {
		protocolVersion: 1,
		clientCapabilities: {},
		clientInfo: { name: "klets-smoke", version: "0.1.0" },
	});
	console.log("initialize ->", short(init));

	const session = await request("session/new", { cwd: process.cwd(), mcpServers: [] });
	console.log("session/new ->", short(session));

	const prompt = await request("session/prompt", {
		sessionId: session.sessionId,
		prompt: [{ type: "text", text: "Reply with exactly: klets ok" }],
	});
	console.log("session/prompt ->", short(prompt));
} catch (error) {
	console.error("FAILED:", error.message);
} finally {
	child.kill();
	process.exit(0);
}
