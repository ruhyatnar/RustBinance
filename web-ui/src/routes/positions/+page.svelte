<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import PositionTable from '$lib/components/PositionTable.svelte';
	import { formatDecimalString } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
</script>

<div class="space-y-4">
	<PositionTable
		positions={snap?.positions ?? []}
		canMutate={botStream.canMutate}
		onClosePosition={async (id) => {
			await botStream.closePosition(id);
		}}
	/>

	<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
		<h2 class="mb-3 text-xs font-bold tracking-wider text-slate-400 uppercase">
			Active Orders ({snap?.orders.length ?? 0})
		</h2>
		{#if !snap || snap.orders.length === 0}
			<p class="py-4 text-center text-xs text-slate-500">No active orders.</p>
		{:else}
			<div class="overflow-x-auto">
				<table class="w-full text-left font-mono text-xs">
					<thead class="border-b border-slate-800 text-slate-400">
						<tr>
							<th class="py-2 pr-3">Client ID</th>
							<th class="py-2 pr-3">Venue</th>
							<th class="py-2 pr-3">Symbol</th>
							<th class="py-2 pr-3">Side / Kind</th>
							<th class="py-2 pr-3">Purpose</th>
							<th class="py-2 pr-3">Price</th>
							<th class="py-2 pr-3">Filled / Qty</th>
							<th class="py-2">Status</th>
						</tr>
					</thead>
					<tbody class="divide-y divide-slate-800/80">
						{#each snap.orders as ord (ord.client_id)}
							<tr>
								<td class="py-2 pr-3 text-slate-300">{ord.client_id}</td>
								<td class="py-2 pr-3 uppercase">{ord.venue}</td>
								<td class="py-2 pr-3 font-semibold text-white">{ord.symbol}</td>
								<td class="py-2 pr-3">{ord.side} · {ord.kind}</td>
								<td class="py-2 pr-3 text-sky-300">{ord.purpose}</td>
								<td class="py-2 pr-3">{ord.price ? formatDecimalString(ord.price) : 'MKT'}</td>
								<td class="py-2 pr-3">
									{formatDecimalString(ord.filled_qty)} / {formatDecimalString(ord.qty)}
								</td>
								<td class="py-2 text-amber-300">{ord.status}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>
</div>
