<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import StatusPill from '$lib/components/ui/StatusPill.svelte';
	import { formatDuration } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);

	const wsTone = $derived(
		botStream.connectionState === 'connected'
			? 'positive'
			: botStream.connectionState === 'connecting' || botStream.connectionState === 'reconnecting'
				? 'warning'
				: 'danger'
	);

	const wsIcon = $derived(
		botStream.connectionState === 'connected' ? '●' : '⚠'
	);
</script>

<div class="space-y-5">
	<PageHeader title="System" subtitle="WebSocket transport, IPC protocol, and clock diagnostics" />

	<Panel title="System & IPC Health" subtitle="Live telemetry" dotGrid={true}>
		<dl class="grid grid-cols-1 gap-3 font-mono text-xs tabular-nums sm:grid-cols-2 lg:grid-cols-3">
			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">WS Connection</dt>
				<dd class="mt-2">
					<StatusPill label={botStream.connectionState} icon={wsIcon} tone={wsTone} />
				</dd>
			</div>

			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">Auth Scope</dt>
				<dd class="mt-2">
					<StatusPill
						label={botStream.authScope}
						icon={botStream.authScope === 'full' ? '🔓' : '🔒'}
						tone={botStream.authScope === 'full' ? 'accent' : 'neutral'}
					/>
				</dd>
			</div>

			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">Protocol Version</dt>
				<dd class="mt-1.5 font-display text-base font-semibold text-ink-strong">
					v{botStream.hello?.protocol_version ?? 1}
				</dd>
			</div>

			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">Engine Instance ID</dt>
				<dd class="mt-1.5 truncate text-ink">
					{botStream.hello?.engine_instance_id ?? '—'}
				</dd>
			</div>

			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">Uptime / Clock Drift</dt>
				<dd class="mt-1.5 text-ink-strong">
					{formatDuration(snap?.engine.uptime_s ?? 0)} · {snap?.engine.clock_drift_ms ?? 0}ms
				</dd>
			</div>

			<div class="rounded border border-hairline-subtle bg-surface-inset/90 p-3.5">
				<dt class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">Last Seq / Gap Resyncs</dt>
				<dd class="mt-1.5 text-ink-strong">
					{botStream.lastSeq ?? '—'} / {botStream.gapResyncCount}
				</dd>
			</div>
		</dl>
	</Panel>
</div>
