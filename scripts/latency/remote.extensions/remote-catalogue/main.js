// The bundled Remote package's catalogue at its largest, 4,000 rows matched by words: four
// hosts' processes, ports, log lines and files, shaped as Remote lists them. Every row is the
// same on every run.
const PART = 400;
const HOSTS = ['web1', 'web2', 'db1', 'build1'];
const COMMANDS = [
	'nginx: worker process',
	'/usr/bin/node /srv/app/server.js --port 3000',
	'postgres: 16/main: checkpointer',
	'/usr/lib/systemd/systemd-journald',
	'sshd: deploy@notty',
	'/usr/sbin/cron -f',
];

function rows() {
	const out = [];
	for (const host of HOSTS) {
		out.push({
			id: `host:${host}`,
			title: host,
			subtitle: '300 processes · 100 ports · 400 log lines · 200 files',
			section: 'Hosts',
			time: 1795683558000,
			accessories: [{ tag: 'online' }],
			matchText: `${host} host`,
		});
	}
	for (const host of HOSTS) {
		for (let pid = 1; pid <= 300; pid++) {
			const command = `${COMMANDS[pid % COMMANDS.length]} ${pid}`;
			out.push({
				id: `ps:${host}:${pid}`,
				title: command,
				subtitle: `${host} · PID ${pid} · www-data · 0.3% CPU · 0.4% memory · up 41-03:11:58`,
				section: `${host} · Processes`,
				time: 1795683558000,
				text: command,
				matchText: `${host} ps processes ${pid} www-data ${command}`,
			});
		}
		for (let index = 0; index < 100; index++) {
			const local = `0.0.0.0:${8000 + index}`;
			out.push({
				id: `port:${host}:${index}`,
				title: `tcp ${local}`,
				subtitle: `${host} · LISTEN · users:(("node",pid=${index},fd=6))`,
				section: `${host} · Ports`,
				time: 1795683558000,
				text: local,
				matchText: `${host} ports port tcp ${local} users:(("node",pid=${index},fd=6))`,
			});
		}
		for (let index = 0; index < 400; index++) {
			const line = `2026-11-26T08:58:${String(index % 60).padStart(2, '0')}+0000 ${host} nginx[813]: request ${index} upstream timed out`;
			out.push({
				id: `log:${host}:${index}`,
				title: line,
				subtitle: `${host} · nginx.service`,
				section: `${host} · nginx.service`,
				time: 1795683558000,
				text: line,
				matchText: `${host} logs log nginx.service ${line}`,
			});
		}
		for (let index = 0; index < 199; index++) {
			const path = `/srv/app/releases/${index}/server.js`;
			out.push({
				id: `file:${host}:${index}`,
				title: 'server.js',
				subtitle: `${host} · /srv/app/releases/${index}`,
				section: `${host} · /srv/app`,
				time: 1795683558000,
				text: path,
				matchText: `${host} files file ${path}`,
			});
		}
	}
	return out;
}

export function items(ctx) {
	const all = rows();
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const end = Math.min(start + PART, all.length);
	const part = all.slice(start, end);
	const more = end < all.length;
	return start === 0
		? { items: part, cursor: String(end), more }
		: { type: 'update', append: part, cursor: String(end), more };
}
