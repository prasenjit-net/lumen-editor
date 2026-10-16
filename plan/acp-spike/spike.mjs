// Throwaway ACP client used to capture real adapter behavior (Phase 0).
// usage: node spike.mjs <name> <command> [--terminal] [--prompt "text"] [--no-fs]
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const [name, command, ...rest] = process.argv.slice(2);
const flag = (f) => rest.includes(f);
const arg = (f) => { const i = rest.indexOf(f); return i >= 0 ? rest[i + 1] : undefined; };
const workspace = fs.realpathSync(arg('--cwd') ?? process.cwd());
const out = fs.createWriteStream(path.join(process.env.TRANSCRIPTS, `${name}.jsonl`));
const t0 = Date.now();
const log = (dir, obj) => out.write(JSON.stringify({ t: Date.now() - t0, dir, msg: obj }) + '\n');

const firstFlag = rest.findIndex((a) => a.startsWith('--'));
const adapterArgs = firstFlag < 0 ? rest : rest.slice(0, firstFlag);
const child = spawn(command, adapterArgs, {
  cwd: workspace, stdio: ['pipe', 'pipe', 'pipe'], env: process.env,
});
child.stderr.on('data', (d) => log('stderr', d.toString()));
let nextId = 1; const pending = new Map();
const send = (obj) => { log('out', obj); child.stdin.write(JSON.stringify(obj) + '\n'); };
const request = (method, params) => new Promise((resolve) => {
  const id = nextId++; pending.set(id, resolve); send({ jsonrpc: '2.0', id, method, params });
});
const permissionLog = [];
const terminals = new Map(); let nextTerminal = 1;
const inside = (p) => p && path.resolve(p).startsWith(workspace + path.sep);

async function handleAgentRequest(m) {
  const reply = (result) => send({ jsonrpc: '2.0', id: m.id, result });
  const fail = (code, message) => send({ jsonrpc: '2.0', id: m.id, error: { code, message } });
  const p = m.params ?? {};
  switch (m.method) {
    case 'fs/read_text_file': {
      if (!inside(p.path)) return fail(-32602, 'outside workspace');
      try { let t = fs.readFileSync(p.path, 'utf8');
        if (p.line || p.limit) { const lines = t.split('\n'); t = lines.slice((p.line ?? 1) - 1, p.limit ? (p.line ?? 1) - 1 + p.limit : undefined).join('\n'); }
        return reply({ content: t }); } catch (e) { return fail(-32002, String(e)); }
    }
    case 'fs/write_text_file': {
      if (!inside(p.path)) return fail(-32602, 'outside workspace');
      fs.writeFileSync(p.path, p.content); return reply(null);
    }
    case 'session/request_permission': {
      const opts = p.options ?? [];
      const allow = opts.find((o) => o.kind === 'allow_once') ?? opts[0];
      const locs = (p.toolCall?.locations ?? []).map((l) => l.path);
      const ok = locs.every(inside);
      permissionLog.push({ title: p.toolCall?.title, kind: p.toolCall?.kind, options: opts.map((o) => `${o.kind}:${o.optionId}`), allowed: ok });
      const reject = opts.find((o) => o.kind === 'reject_once');
      return reply({ outcome: ok && allow ? { outcome: 'selected', optionId: allow.optionId } : reject ? { outcome: 'selected', optionId: reject.optionId } : { outcome: 'cancelled' } });
    }
    case 'terminal/create': {
      const id = `t${nextTerminal++}`;
      const proc = spawn(p.command, p.args ?? [], { cwd: p.cwd ?? workspace, shell: (p.args ?? []).length === 0, env: { ...process.env, ...Object.fromEntries((p.env ?? []).map((e) => [e.name, e.value])) } });
      const t = { output: '', exit: null, waiters: [] };
      const add = (d) => { t.output += d.toString(); };
      proc.stdout.on('data', add); proc.stderr.on('data', add);
      proc.on('exit', (code, signal) => { t.exit = { exitCode: code, signal }; t.waiters.forEach((w) => w(t.exit)); });
      t.proc = proc; terminals.set(id, t); return reply({ terminalId: id });
    }
    case 'terminal/output': { const t = terminals.get(p.terminalId); return reply({ output: t.output, truncated: false, ...(t.exit ? { exitStatus: t.exit } : {}) }); }
    case 'terminal/wait_for_exit': { const t = terminals.get(p.terminalId); return t.exit ? reply(t.exit) : t.waiters.push(reply); }
    case 'terminal/kill': { terminals.get(p.terminalId)?.proc.kill(); return reply(null); }
    case 'terminal/release': { terminals.delete(p.terminalId); return reply(null); }
    default: return fail(-32601, `Method not supported by spike: ${m.method}`);
  }
}

let buffer = '';
child.stdout.on('data', (d) => {
  buffer += d.toString();
  let i;
  while ((i = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, i); buffer = buffer.slice(i + 1);
    if (!line.trim()) continue;
    let m; try { m = JSON.parse(line); } catch { log('bad', line); continue; }
    log('in', m);
    if (m.id !== undefined && m.method) handleAgentRequest(m);
    else if (m.id !== undefined && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  }
});
child.on('exit', (code, sig) => log('exit', { code, sig }));

const summary = { name, workspace };
const deadline = setTimeout(() => { summary.timeout = true; finish(); }, Number(arg('--timeout') ?? 150000));
async function finish() {
  summary.permissions = permissionLog;
  fs.writeFileSync(path.join(process.env.TRANSCRIPTS, `${name}.summary.json`), JSON.stringify(summary, null, 2));
  clearTimeout(deadline); child.stdin.end(); setTimeout(() => { child.kill(); out.end(); process.exit(0); }, 800);
}

const init = await request('initialize', {
  protocolVersion: 1,
  clientCapabilities: { fs: flag('--no-fs') ? { readTextFile: false, writeTextFile: false } : { readTextFile: true, writeTextFile: true }, terminal: flag('--terminal') },
  clientInfo: { name: 'lumen-spike', title: 'Lumen ACP spike', version: '0.0.0' },
});
summary.initialize = init.result ?? init.error;
const created = await request('session/new', { cwd: workspace, mcpServers: [] });
summary.sessionNew = created.result ?? created.error;
if (created.error && arg('--auth')) {
  summary.authenticate = await request('authenticate', { methodId: arg('--auth') });
}
const prompt = arg('--prompt');
const sid = created.result?.sessionId;
for (const setting of rest.flatMap((a, i) => (a === '--set' ? [rest[i + 1]] : []))) {
  const [configId, value] = setting.split('=');
  summary[`set_${configId}`] = (await request('session/set_config_option', { sessionId: sid, configId, value })).result?.configOptions?.map((o) => `${o.id}=${o.currentValue}`) ?? 'error';
}
if (flag('--list')) summary.sessionList = (await request('session/list', {})).result ?? 'error';
if (arg('--cancel-after') && prompt && sid) {
  setTimeout(() => { send({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId: sid } }); }, Number(arg('--cancel-after')));
}
if (prompt && created.result?.sessionId) {
  const done = await request('session/prompt', { sessionId: created.result.sessionId, prompt: [{ type: 'text', text: prompt }] });
  summary.promptResult = done.result ?? done.error;
}
await finish();
