<script lang="ts">
	import type { LoggedEvent } from '$lib/api/stream.svelte';
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
				return 'bg-red-500/20 text-red-300';
			case 'warn':
				return 'bg-amber-500/20 text-amber-300';
			default:
				return 'bg-sky-500/15 text-sky-300';
		}
	}
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-3 flex items-center justify-between">
		<h2 class="text-xs font-bold tracking-wider text-slate-400 uppercase">Events</h2>
		<span class="text-xs text-slate-500">{events.length} logged</span>
	</div>

	{#if visible.length === 0}
		<p class="py-4 text-center text-xs text-slate-500">Waiting for engine events…</p>
	{:else}
		<ul class="max-h-64 space-y-1.5 overflow-y-auto font-mono text-xs">
			{#each visible as ev (`${ev.seq}-${ev.ts_ms}`)}
				<li class="flex flex-wrap items-center gap-2 rounded bg-slate-950/60 px-2.5 py-1.5">
					<span class="text-slate-500">{formatUtcTime(ev.ts_ms)}</span>
					<span class={`rounded px-1.5 py-0.5 text-[10px] font-bold uppercase ${levelBadge(ev.level)}`}>
						{ev.level}
					</span>
					{#if ev.venue}
						<span class="rounded bg-slate-800 px-1 py-0.5 text-[10px] uppercase text-slate-300">
							{ev.venue}
						</span>
					{/if}
					{#if ev.symbol}
						<span class="font-semibold text-white">{ev.symbol}</span>
					{/if}
					<span class="text-slate-300">{ev.kind}:</span>
					<span class="text-slate-200">{ev.message}</span>
				</li>
			{/each}
		</ul>
	{/if}
</section>
