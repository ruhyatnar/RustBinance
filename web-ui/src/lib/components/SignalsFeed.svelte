<script lang="ts">
	import type { ShadowSignalView } from '$lib/api/types';
	import { formatDecimalString, formatUtcTime } from '$lib/format/decimal';

	interface Props {
		signals: ShadowSignalView[];
	}

	let { signals }: Props = $props();
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-3 flex items-center justify-between">
		<h2 class="text-xs font-bold tracking-wider text-slate-400 uppercase">
			Signals (Shadow / Live)
		</h2>
		<span class="text-xs text-slate-500">{signals.length} items</span>
	</div>

	{#if signals.length === 0}
		<p class="py-4 text-center text-xs text-slate-500">No recent signals.</p>
	{:else}
		<ul class="divide-y divide-slate-800/80 font-mono text-xs">
			{#each signals as sig, idx (`${sig.symbol}-${sig.ts_ms}-${idx}`)}
				<li class="flex flex-wrap items-center justify-between gap-2 py-2">
					<div class="flex items-center gap-2">
						<span class="text-slate-500">{formatUtcTime(sig.ts_ms)}</span>
						<span class="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-bold uppercase text-slate-300">
							{sig.venue}
						</span>
						<span class="font-semibold text-white">{sig.symbol}</span>
						<span class="text-sky-300">{sig.strategy}</span>
						<span class="rounded bg-indigo-500/20 px-1.5 py-0.5 text-[10px] text-indigo-200">
							{sig.direction}
						</span>
					</div>
					<div class="text-slate-300">
						Entry {formatDecimalString(sig.entry)} · SL {formatDecimalString(sig.stop)}
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</section>
