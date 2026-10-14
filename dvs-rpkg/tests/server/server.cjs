// A real dvs-server for the R package's server tests, and the few things those tests
// need that R does not do (approving a login, creating, locking and yanking).
//
//   node server.cjs serve -- <command…>
//       Start dvs-server on a scratch database with the dev personas (alice, bob,
//       carol; tokens tok-<name>) behind a stand-in for the edge, run the command
//       with DVS_TEST_SERVER set to the edge's URL, then stop and clean up. Exits
//       with the command's status.
//   node server.cjs approve <verification uri> <user>
//   node server.cjs create <url> <user> <project> <admin group>
//   node server.cjs lock <url> <user> <project> <reason>
//   node server.cjs yank <url> <user> <project> <hash> <reason>
//
// Env for serve: DATABASE_URL (a Postgres server where you can create databases),
// DVS_SERVER_ROOT (a dvs-server checkout: templates, docker/arx/users.json),
// DVS_SERVER_BIN (default: $DVS_SERVER_ROOT/target/debug/dvs-server, built with
// --features debug), PSQL (default: psql). Node 18 or newer; no packages.
'use strict';
const { spawn, execFileSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.once('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

function psql(url, sql) {
  execFileSync(process.env.PSQL || 'psql', [url, '-v', 'ON_ERROR_STOP=1', '-qc', sql], { stdio: 'inherit' });
}

// What the edge does in front of the server: add the trusted proxy secret and say how
// the client reached it.
function startEdge(base, secret) {
  const target = new URL(base);
  const edge = http.createServer((req, res) => {
    const headers = {
      ...req.headers,
      host: target.host,
      'x-dvs-proxy-secret': secret,
      'x-forwarded-host': req.headers.host,
      'x-forwarded-proto': 'http',
    };
    const upstream = http.request(
      { host: target.hostname, port: target.port, method: req.method, path: req.url, headers },
      reply => {
        res.writeHead(reply.statusCode, reply.headers);
        reply.pipe(res);
      },
    );
    upstream.on('error', e => res.destroy(e));
    req.pipe(upstream);
  });
  // Longer than the device-login poll interval, as real proxies are
  edge.keepAliveTimeout = 65000;
  return new Promise(resolve => edge.listen(0, '127.0.0.1', () => resolve(edge)));
}

async function waitFor(base, secret, server) {
  for (let i = 0; i < 200; i++) {
    if (server.exitCode !== null) throw new Error(`the server exited with ${server.exitCode}`);
    try {
      const r = await fetch(`${base}/api/server`, { headers: { 'x-dvs-proxy-secret': secret } });
      if (r.ok) return;
    } catch {
      // not listening yet
    }
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('the server did not start within 20s');
}

async function serve(command) {
  const admin = process.env.DATABASE_URL;
  const root = process.env.DVS_SERVER_ROOT;
  if (!admin) throw new Error('set DATABASE_URL to a Postgres server where you can create databases');
  if (!root) throw new Error('set DVS_SERVER_ROOT to a dvs-server checkout');
  const bin = process.env.DVS_SERVER_BIN || path.join(root, 'target/debug/dvs-server');
  if (!fs.existsSync(bin)) throw new Error(`${bin} is missing: build it with --features debug`);
  if (command.length === 0) throw new Error('give the command to run after --');

  const id = crypto.randomBytes(4).toString('hex');
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'dvs-rpkg-server-'));
  fs.mkdirSync(path.join(work, 'blobs'));
  const dbName = `dvs_rpkg_${id}`;
  const dbUrl = new URL(admin);
  dbUrl.pathname = `/${dbName}`;
  const port = await freePort();
  const secret = crypto.randomBytes(16).toString('hex');
  const base = `http://127.0.0.1:${port}`;
  fs.writeFileSync(
    path.join(work, 'server.toml'),
    [
      `port = ${port}`,
      '[mode.local_disk]',
      `storage = ${JSON.stringify(path.join(work, 'blobs'))}`,
      '[groups]',
      'resolver = "arx"',
      'admin_group = "dvs-admins"',
      '',
    ].join('\n'),
  );

  psql(admin, `CREATE DATABASE ${dbName}`);
  const env = { ...process.env, DATABASE_URL: dbUrl.toString(), TRUSTED_PROXY_SECRET: secret };
  let server;
  let edge;
  let status = 1;
  try {
    execFileSync(bin, ['debug', 'seed', '--users-file', path.join(root, 'docker/arx/users.json')], {
      env,
      stdio: 'ignore',
    });
    const log = fs.openSync(path.join(work, 'server.log'), 'w');
    server = spawn(bin, ['serve', '--config-file', path.join(work, 'server.toml')], {
      cwd: root,
      env,
      stdio: ['ignore', log, log],
    });
    await waitFor(base, secret, server);
    edge = await startEdge(base, secret);
    const url = `http://127.0.0.1:${edge.address().port}/`;
    console.error(`dvs-server at ${url} (log: ${path.join(work, 'server.log')})`);
    // The command runs asynchronously: the edge serves from this process
    const child = spawn(command[0], command.slice(1), {
      stdio: 'inherit',
      env: { ...process.env, DVS_TEST_SERVER: url },
    });
    status = await new Promise(resolve => child.on('exit', code => resolve(code ?? 1)));
  } finally {
    if (edge) edge.close();
    if (server && server.exitCode === null) {
      server.kill('SIGTERM');
      await new Promise(r => server.once('exit', r));
    }
    psql(admin, `DROP DATABASE ${dbName} WITH (FORCE)`);
    if (status === 0) fs.rmSync(work, { recursive: true, force: true });
    else console.error(`kept ${work}`);
  }
  return status;
}

function api(url, user, method, route, body) {
  return fetch(new URL(route, url), {
    method,
    headers: { authorization: `Bearer tok-${user}`, 'content-type': 'application/json' },
    body: body && JSON.stringify(body),
  }).then(async r => {
    const text = await r.text();
    if (!r.ok) throw new Error(`${method} ${route}: ${r.status} ${text}`);
    return text;
  });
}

// Approve a device login in the browser's place, as `user`.
async function approve(uri, user) {
  const who = { 'auth-username': user, 'auth-email': `${user}@example.com` };
  const page = await fetch(uri, { headers: who });
  const html = await page.text();
  const csrf = html.match(/name="csrf_token" value="([^"]+)"/)?.[1];
  const code = html.match(/name="user_code" value="([^"]+)"/)?.[1];
  if (!csrf || !code) throw new Error(`no approval form at ${uri}: ${page.status} ${html.slice(0, 300)}`);
  const cookie = (page.headers.getSetCookie?.() || []).map(c => c.split(';')[0]).join('; ');
  const done = await fetch(new URL('/device/approve', uri), {
    method: 'POST',
    headers: { ...who, cookie, 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ csrf_token: csrf, user_code: code, decision: 'approve' }),
    redirect: 'manual',
  });
  if (done.status >= 400) throw new Error(`approving: ${done.status} ${await done.text()}`);
}

async function main() {
  const [cmd, ...args] = process.argv.slice(2);
  switch (cmd) {
    case 'serve': {
      const dash = args.indexOf('--');
      process.exit(await serve(dash < 0 ? args : args.slice(dash + 1)));
    }
    case 'approve':
      return approve(args[0], args[1]);
    case 'create': {
      const [url, user, project, group] = args;
      return api(url, user, 'POST', 'api/v1/projects', { name: project, grants: [{ group, role: 'admin' }] });
    }
    case 'lock': {
      const [url, user, project, reason] = args;
      return api(url, user, 'PATCH', `api/v1/projects/${project}`, { locked: true, lock_reason: reason });
    }
    case 'yank': {
      const [url, user, project, hash, reason] = args;
      return api(url, user, 'POST', `api/v1/projects/${project}/yanks`, { hash, reason });
    }
    default:
      throw new Error(`unknown command ${cmd}`);
  }
}

main().catch(e => {
  console.error(e.message || e);
  process.exit(1);
});
