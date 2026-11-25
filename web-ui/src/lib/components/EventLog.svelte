<script lang="ts">
	import type { LoggedEvent } from '$lib/api/stream.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import { formatUtcTime } from '$lib/format/decimal';

	interface Props {
		events: LoggedEvent[];
		limit?: number;
	}

	let { events, limit = 50 }: Props = $props();

	const visible = $derived(events.slice(0, limit));

	function levelBadge(level: LoggedEvent['level']): string {
		switch (level) {
			case 'critical':
			case 'error':
				return 'border-danger-solid/60 bg-danger-solid/15 text-danger';
			case 'warn':
				return 'border-warning/40 bg-warning/10 text-warning';
			default:
				return 'border-telemetry/40 bg-telemetry/10 text-telemetry';
		}
	}
</script>

<Panel title="Event Telemetry" subtitle={`${events.length} logged`} dotGrid={true}>
	{#if visible.length === 0}
		<p class="py-5 text-center font-mono text-xs text-ink-dim">Waiting for engine events…</p>
	{:else}
		<ul class="max-h-72 space-y-1.5 overflow-y-auto font-mono text-xs tabular-nums">
			{#each visible as ev (`${ev.seq}-${ev.ts_ms}`)}
				<li
					class="flex flex-wrap items-center gap-2 rounded border border-hairline-subtle bg-surface-inset/90 px-3 py-1.5"
				>
					<span class="text-ink-dim">{formatUtcTime(ev.ts_ms)}</span>
					<span class={`rounded border px-1.5 py-0.5 text-[10px] font-bold uppercase ${levelBadge(ev.level)}`}>
						{ev.level}
					</span>
					{#if ev.venue}
						<span class="rounded border border-hairline-strong bg-surface px-1.5 py-0.5 text-[10px] text-ink-muted uppercase">
							{ev.venue}
						</span>
					{/if}
					{#if ev.symbol}
						<span class="font-semibold text-ink-strong">{ev.symbol}</span>
					{/if}
					<span class="text-ink-muted">{ev.kind}:</span>
					<span class="text-ink">{ev.message}</span>
				</li>
			{/each}
		</ul>
	{/if}
</Panel>
