<script lang="ts">
	import type { ShadowSignalView } from '$lib/api/types';
	import Panel from '$lib/components/ui/Panel.svelte';
	import { formatDecimalString, formatUtcTime } from '$lib/format/decimal';

	interface Props {
		signals: ShadowSignalView[];
	}

	let { signals }: Props = $props();
</script>

<Panel title="Signals (Shadow / Live)" subtitle={`${signals.length} items`} dotGrid={true}>
	{#if signals.length === 0}
		<p class="py-5 text-center font-mono text-xs text-ink-dim">No recent signals.</p>
	{:else}
		<ul class="space-y-2 font-mono text-xs tabular-nums">
			{#each signals as sig, idx (`${sig.symbol}-${sig.ts_ms}-${idx}`)}
				<li
					class="flex flex-wrap items-center justify-between gap-2 rounded border border-hairline-subtle bg-surface-inset/90 px-3 py-2"
				>
					<div class="flex flex-wrap items-center gap-2">
						<span class="text-ink-dim">{formatUtcTime(sig.ts_ms)}</span>
						<span class="rounded border border-hairline-strong bg-surface px-1.5 py-0.5 text-[10px] font-bold text-ink-muted uppercase">
							{sig.venue}
						</span>
						<span class="font-semibold text-ink-strong">{sig.symbol}</span>
						<span class="text-telemetry">{sig.strategy}</span>
						<span class="rounded border border-accent/40 bg-accent/15 px-1.5 py-0.5 text-[10px] text-accent-soft">
							{sig.direction}
						</span>
					</div>
					<div class="text-ink-muted">
						Entry <span class="text-ink-strong">{formatDecimalString(sig.entry)}</span>
						· SL <span class="text-danger">{formatDecimalString(sig.stop)}</span>
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</Panel>
