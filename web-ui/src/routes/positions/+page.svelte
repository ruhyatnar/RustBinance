<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import PositionTable from '$lib/components/PositionTable.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import StatusPill from '$lib/components/ui/StatusPill.svelte';
	import { filterByVenue, type VenueFilter } from '$lib/filters/venue';
	import { formatDecimalString } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
	let venueFilter = $state<VenueFilter>('all');

	const filteredPositions = $derived(
		snap ? filterByVenue(snap.positions, venueFilter) : []
	);
	const filteredOrders = $derived(
		snap ? filterByVenue(snap.orders, venueFilter) : []
	);
</script>

<div class="space-y-5">
	<PageHeader title="Positions & Orders" subtitle="Open exposure and active exchange orders">
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
		{/snippet}
	</PageHeader>

	<PositionTable
		positions={filteredPositions}
		canMutate={botStream.canMutate}
		onClosePosition={async (id) => {
			await botStream.closePosition(id);
		}}
	/>

	<Panel title={`Active Orders (${filteredOrders.length})`} padding="none">
		{#if filteredOrders.length === 0}
			<p class="py-8 text-center font-mono text-xs text-ink-dim">No active orders.</p>
		{:else}
			<div class="overflow-x-auto">
				<table class="w-full text-left font-mono text-xs tabular-nums">
					<thead class="border-b border-hairline-subtle bg-surface-inset/70 text-[10px] tracking-[0.08em] text-ink-muted uppercase">
						<tr>
							<th class="px-4 py-2.5">Client ID</th>
							<th class="px-3 py-2.5">Venue</th>
							<th class="px-3 py-2.5">Symbol</th>
							<th class="px-3 py-2.5">Side / Kind</th>
							<th class="px-3 py-2.5">Purpose</th>
							<th class="px-3 py-2.5">Price</th>
							<th class="px-3 py-2.5">Filled / Qty</th>
							<th class="px-4 py-2.5 text-right">Status</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-hairline-subtle">
						{#each filteredOrders as ord (ord.client_id)}
							<tr class="transition hover:bg-surface-elevated/60">
								<td class="px-4 py-2.5 text-ink-muted">{ord.client_id}</td>
								<td class="px-3 py-2.5">
									<span class="rounded border border-hairline-strong bg-surface-inset px-1.5 py-0.5 text-[10px] font-bold text-ink uppercase">
										{ord.venue}
									</span>
								</td>
								<td class="px-3 py-2.5 font-semibold text-ink-strong">{ord.symbol}</td>
								<td class="px-3 py-2.5 text-ink">{ord.side} · {ord.kind}</td>
								<td class="px-3 py-2.5 text-telemetry">{ord.purpose}</td>
								<td class="px-3 py-2.5 text-ink-strong">
									{ord.price ? formatDecimalString(ord.price) : 'MKT'}
								</td>
								<td class="px-3 py-2.5 text-ink">
									{formatDecimalString(ord.filled_qty)} / {formatDecimalString(ord.qty)}
								</td>
								<td class="px-4 py-2.5 text-right">
									<StatusPill label={ord.status} tone="warning" />
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</Panel>
</div>
