import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

export class NativeConnection {
  constructor(executable, path, audit) {
    this.audit = audit;
    this.pending = [];
    this.child = spawn(executable, [path], { stdio: ['pipe', 'pipe', 'pipe'] });
    this.stderr = '';
    this.child.stderr.on('data', value => { this.stderr += value; });
    createInterface({ input: this.child.stdout }).on('line', line => {
      const pending = this.pending.shift();
      if (!pending) return;
      clearTimeout(pending.timer);
      try {
        const reply = JSON.parse(line);
        this.audit.push({ sql: pending.sql, ...reply });
        if (reply.ok) pending.resolve(reply.rows);
        else pending.reject(new Error(reply.error));
      } catch (error) { pending.reject(error); }
    });
    this.closed = new Promise(resolve => this.child.once('close', (code, signal) => {
      this.exitCode = code;
      for (const pending of this.pending.splice(0)) {
        clearTimeout(pending.timer);
        pending.reject(new Error(`native process exited ${code}/${signal}: ${this.stderr}`));
      }
      resolve(code);
    }));
    this.child.on('error', error => {
      for (const pending of this.pending.splice(0)) {
        clearTimeout(pending.timer);
        pending.reject(error);
      }
    });
  }
  query(sql, params) {
    return new Promise((resolve, reject) => {
      if (this.exitCode !== undefined) return reject(new Error(this.stderr || 'process already closed'));
      const timer = setTimeout(() => this.child.kill('SIGKILL'), 60000);
      this.pending.push({ resolve, reject, timer, sql });
      this.child.stdin.write(JSON.stringify({ sql, ...(params === undefined ? {} : { params }) }) + '\n');
    });
  }
  async close() {
    this.child.stdin.end();
    await this.closed;
    if (this.exitCode !== 0) throw new Error(`native exit ${this.exitCode}: ${this.stderr}`);
  }
}

export const literal = value => JSON.stringify(value);
export const unique = values => [...new Set(values)];
export function tokens(text) {
  // Identical deterministic Chinese bigrams + Latin tokens in all three native FTS engines.
  const result = [];
  for (const part of text.toLowerCase().match(/[\p{Script=Han}]+|[a-z0-9_]+/gu) || []) {
    if (/\p{Script=Han}/u.test(part)) {
      const chars = [...part];
      if (chars.length === 1) result.push(part);
      else for (let i = 0; i < chars.length - 1; ++i) result.push(chars[i] + chars[i + 1]);
    } else result.push(part);
  }
  return result;
}
