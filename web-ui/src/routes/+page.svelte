<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import Candles from '$lib/charts/Candles.svelte';
	import EventLog from '$lib/components/EventLog.svelte';
	import PositionTable from '$lib/components/PositionTable.svelte';
	import RiskGauge from '$lib/components/RiskGauge.svelte';
	import SignalsFeed from '$lib/components/SignalsFeed.svelte';

	const snap = $derived(botStream.snapshot);
	const primaryPos = $derived(snap?.positions[0] ?? null);
	const chartSymbol = $derived(primaryPos?.symbol ?? 'BTCUSDT');

	async function handlePause(): Promise<void> {
		await botStream.pause();
	}

	async function handleResume(): Promise<void> {
		await botStream.resume();
	}

	async function handleClosePosition(id: string): Promise<void> {
		await botStream.closePosition(id);
	}
</script>

{#if !snap}
	<div class="rounded-xl border border-slate-800 bg-slate-900/60 p-8 text-center text-sm text-slate-400">
		Waiting for initial IPC snapshot… ({botStream.connectionState})
	</div>
{:else}
	<div class="space-y-4">
		{#if botStream.canMutate}
			<div class="flex flex-wrap items-center gap-2">
				<button
					type="button"
					onclick={handlePause}
					class="rounded-md border border-amber-500/60 bg-amber-500/15 px-3 py-1.5 text-xs font-semibold text-amber-200 hover:bg-amber-500/25"
				>
					❚❚ Pause Entries
				</button>
				<button
					type="button"
					onclick={handleResume}
					class="rounded-md border border-emerald-500/60 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-200 hover:bg-emerald-500/25"
				>
					▶ Resume Entries
				</button>
			</div>
		{/if}

		<div class="grid grid-cols-1 gap-4 lg:grid-cols-12">
			<div class="space-y-4 lg:col-span-4">
				<RiskGauge risk={snap.risk} />
				<SignalsFeed signals={snap.signals_shadow} />
			</div>

			<div class="space-y-4 lg:col-span-8">
				<PositionTable
					positions={snap.positions}
					canMutate={botStream.canMutate}
					onClosePosition={handleClosePosition}
				/>
				<Candles
					symbol={chartSymbol}
					timeframe="1h"
					bars={[]}
					entryPrice={primaryPos?.entry ?? null}
					stopPrice={primaryPos?.stop ?? null}
					tp1Price={primaryPos?.tp1 ?? null}
				/>
			</div>
		</div>

		<EventLog events={botStream.events} limit={25} />
	</div>
{/if}
