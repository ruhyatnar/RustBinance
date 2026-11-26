<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import Candles from '$lib/charts/Candles.svelte';
	import EventLog from '$lib/components/EventLog.svelte';
	import PositionTable from '$lib/components/PositionTable.svelte';
	import RiskGauge from '$lib/components/RiskGauge.svelte';
	import SignalsFeed from '$lib/components/SignalsFeed.svelte';
	import KpiCard from '$lib/components/ui/KpiCard.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import { filterByVenue, filterEventsByVenue, type VenueFilter } from '$lib/filters/venue';
	import { decimalDirectionIcon, decimalSign, formatDecimalString } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
	let venueFilter = $state<VenueFilter>('all');

	const filteredPositions = $derived(
		snap ? filterByVenue(snap.positions, venueFilter) : []
	);
	const filteredSignals = $derived(
		snap ? filterByVenue(snap.signals_shadow, venueFilter) : []
	);
	const filteredEvents = $derived(
		filterEventsByVenue(botStream.events, venueFilter)
	);

	const primaryPos = $derived(filteredPositions[0] ?? null);
	const chartSymbol = $derived(primaryPos?.symbol ?? 'BTCUSDT');

	function clampPct(numerator: string, denominator: string): number {
		const n = Math.abs(Number(numerator));
		const d = Math.abs(Number(denominator));
		if (!Number.isFinite(n) || !Number.isFinite(d) || d <= 0) return 0;
		return Math.min(100, Math.round((n / d) * 100));
	}

	const daySign = $derived(snap ? decimalSign(snap.risk.day_pnl_pct) : 0);
	const heatPct = $derived(
		snap ? clampPct(snap.risk.heat_pct, snap.risk.heat_max_pct) : 0
	);
	const ddPct = $derived(
		snap ? clampPct(snap.risk.drawdown_pct, snap.risk.limits.kill_pct) : 0
	);

	let actionFeedback = $state<{ message: string; isError: boolean } | null>(null);

	async function handlePause(): Promise<void> {
		try {
			const res = await botStream.pause();
			actionFeedback = { message: res.message || 'Entries paused', isError: !res.ok };
		} catch (err) {
			actionFeedback = {
				message: err instanceof Error ? err.message : 'Failed to pause entries',
				isError: true
			};
		}
	}

	async function handleResume(): Promise<void> {
		try {
			const res = await botStream.resume();
			actionFeedback = { message: res.message || 'Entries resumed', isError: !res.ok };
		} catch (err) {
			actionFeedback = {
				message: err instanceof Error ? err.message : 'Failed to resume entries',
				isError: true
			};
		}
	}

	async function handleClosePosition(id: string): Promise<void> {
		try {
			const res = await botStream.closePosition(id);
			actionFeedback = {
				message: res.message || `Close requested for ${id}`,
				isError: !res.ok
			};
		} catch (err) {
			actionFeedback = {
				message: err instanceof Error ? err.message : `Failed to close position ${id}`,
				isError: true
			};
		}
	}
</script>

{#if !snap}
	<Panel dotGrid={true}>
		<div class="py-10 text-center font-mono text-xs text-ink-muted">
			Waiting for initial IPC snapshot… ({botStream.connectionState})
		</div>
	</Panel>
{:else}
	<div class="space-y-5">
		<PageHeader title="Cockpit" subtitle="Real-time execution & risk telemetry">
			{#snippet actions()}
				<div
					class="inline-flex items-center rounded border border-hairline bg-surface-inset p-0.5 font-mono text-[11px]"
					role="group"
					aria-label="Filter venue"
				>
					{#each [
						{ id: 'all', label: 'All' },
						{ id: 'spot', label: 'Spot' },
						{ id: 'futures', label: 'Futures' }
					] as opt (opt.id)}
						<button
							type="button"
							aria-pressed={venueFilter === opt.id}
							onclick={() => (venueFilter = opt.id as VenueFilter)}
							class={`rounded px-2.5 py-1 font-semibold transition ${
								venueFilter === opt.id
									? 'bg-surface-elevated text-ink-strong ring-1 ring-accent/50'
									: 'text-ink-muted hover:text-ink'
							}`}
						>
							{opt.label}
						</button>
					{/each}
				</div>

				{#if botStream.canMutate}
					<button
						type="button"
						onclick={handlePause}
						class="rounded border border-warning/50 bg-warning/10 px-3 py-1.5 font-mono text-[11px] font-semibold text-warning transition hover:bg-warning/20"
					>
						❚❚ Pause Entries
					</button>
					<button
						type="button"
						onclick={handleResume}
						class="rounded border border-positive/50 bg-positive/10 px-3 py-1.5 font-mono text-[11px] font-semibold text-positive transition hover:bg-positive/20"
					>
						▶ Resume Entries
					</button>
				{/if}
			{/snippet}
		</PageHeader>

		{#if actionFeedback}
			<div
				role={actionFeedback.isError ? 'alert' : 'status'}
				class={`flex items-center justify-between rounded border px-3 py-2 font-mono text-xs ${
					actionFeedback.isError
						? 'border-danger-solid/50 bg-danger-solid/10 text-danger'
						: 'border-warning/40 bg-warning/10 text-warning'
				}`}
			>
				<span>{actionFeedback.message}</span>
				<button
					type="button"
					onclick={() => (actionFeedback = null)}
					class="rounded border border-current/40 px-2 py-0.5 text-[11px] hover:opacity-80"
				>
					Dismiss
				</button>
			</div>
		{/if}

		<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
			<KpiCard
				label="Equity"
				value={formatDecimalString(snap.risk.equity, { decimals: 2 })}
				sublabel="Account Aggregate (USDT)"
				tone="neutral"
			/>
			<KpiCard
				label="Day PnL"
				value={formatDecimalString(snap.risk.day_pnl_pct, { signed: true, suffix: '%' })}
				icon={decimalDirectionIcon(snap.risk.day_pnl_pct)}
				sublabel={`Halt limit ${formatDecimalString(snap.risk.limits.daily_halt_pct, { suffix: '%' })} · Account`}
				tone={daySign > 0 ? 'positive' : daySign < 0 ? 'danger' : 'neutral'}
			/>
			<KpiCard
				label="Portfolio Heat"
				value={`${formatDecimalString(snap.risk.heat_pct)}%`}
				sublabel={`Max ${formatDecimalString(snap.risk.heat_max_pct)}% · ${snap.risk.open_positions}/${snap.risk.max_positions} pos`}
				tone={heatPct >= 80 ? 'danger' : heatPct >= 50 ? 'warning' : 'telemetry'}
				progressPct={heatPct}
			/>
			<KpiCard
				label="Drawdown"
				value={formatDecimalString(snap.risk.drawdown_pct, { suffix: '%' })}
				sublabel={`Kill limit ${formatDecimalString(snap.risk.limits.kill_pct, { suffix: '%' })} · Account`}
				tone={ddPct >= 70 ? 'danger' : 'warning'}
				progressPct={ddPct}
			/>
		</div>

		<div class="grid grid-cols-1 gap-4 xl:grid-cols-12">
			<div class="space-y-4 xl:col-span-8">
				<PositionTable
					positions={filteredPositions}
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

			<div class="space-y-4 xl:col-span-4">
				<RiskGauge risk={snap.risk} />
				<SignalsFeed signals={filteredSignals} />
			</div>
		</div>

		<EventLog events={filteredEvents} limit={25} />
	</div>
{/if}
