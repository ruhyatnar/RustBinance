<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import { formatDuration } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4 font-mono text-xs">
	<h2 class="mb-3 font-sans font-bold tracking-wider text-slate-400 uppercase">
		System & IPC Health
	</h2>
	<dl class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">WS Connection</dt>
			<dd class="mt-1 font-bold text-white uppercase">{botStream.connectionState}</dd>
		</div>
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">Auth Scope</dt>
			<dd class="mt-1 font-bold text-sky-300 uppercase">{botStream.authScope}</dd>
		</div>
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">Protocol Version</dt>
			<dd class="mt-1 font-bold text-white">v{botStream.hello?.protocol_version ?? 1}</dd>
		</div>
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">Engine Instance ID</dt>
			<dd class="mt-1 truncate text-slate-200">{botStream.hello?.engine_instance_id ?? '—'}</dd>
		</div>
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">Uptime / Clock Drift</dt>
			<dd class="mt-1 text-slate-200">
				{formatDuration(snap?.engine.uptime_s ?? 0)} · {snap?.engine.clock_drift_ms ?? 0}ms
			</dd>
		</div>
		<div class="rounded bg-slate-950/70 p-3">
			<dt class="text-slate-400">Last Seq / Gap Resyncs</dt>
			<dd class="mt-1 text-slate-200">
				{botStream.lastSeq ?? '—'} / {botStream.gapResyncCount}
			</dd>
		</div>
	</dl>
</section>
