<script lang="ts">
	import type { PositionView } from '$lib/api/types';
	import {
		decimalDirectionIcon,
		decimalSign,
		formatDecimalString,
		formatDuration
	} from '$lib/format/decimal';

	interface Props {
		positions: PositionView[];
		canMutate: boolean;
		onClosePosition: (positionId: string) => void | Promise<void>;
	}

	let { positions, canMutate, onClosePosition }: Props = $props();
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-3 flex items-center justify-between">
		<h2 class="text-xs font-bold tracking-wider text-slate-400 uppercase">
			Positions ({positions.length})
		</h2>
	</div>

	{#if positions.length === 0}
		<p class="py-6 text-center text-xs text-slate-500">No open positions.</p>
	{:else}
		<div class="overflow-x-auto">
			<table class="w-full text-left font-mono text-xs">
				<thead class="border-b border-slate-800 text-slate-400">
					<tr>
						<th class="py-2 pr-3">Symbol</th>
						<th class="py-2 pr-3">Venue</th>
						<th class="py-2 pr-3">PnL (R)</th>
						<th class="py-2 pr-3">Unrealized</th>
						<th class="py-2 pr-3">Entry / Mark</th>
						<th class="py-2 pr-3">SL / TP1</th>
						<th class="py-2 pr-3">Age</th>
						{#if canMutate}
							<th class="py-2 text-right">Action</th>
						{/if}
					</tr>
				</thead>
				<tbody class="divide-y divide-slate-800/80">
					{#each positions as pos (pos.id)}
						{@const sign = decimalSign(pos.profit_r)}
						<tr class="hover:bg-slate-800/40">
							<td class="py-2 pr-3 font-semibold text-white">
								<div>{pos.symbol}</div>
								<div class="text-[10px] font-normal text-slate-400">
									{pos.strategy} · {pos.state}
								</div>
							</td>
							<td class="py-2 pr-3">
								<span class="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-bold uppercase text-slate-200">
									{pos.venue.toUpperCase()}
								</span>
							</td>
							<td
								class={`py-2 pr-3 font-bold ${
									sign > 0 ? 'text-emerald-400' : sign < 0 ? 'text-red-400' : 'text-slate-300'
								}`}
							>
								{decimalDirectionIcon(pos.profit_r)}
								{formatDecimalString(pos.profit_r, { signed: true, suffix: 'R' })}
							</td>
							<td class="py-2 pr-3">
								{formatDecimalString(pos.unrealized_quote, { signed: true })}
							</td>
							<td class="py-2 pr-3">
								<div>{formatDecimalString(pos.entry)}</div>
								<div class="text-[10px] text-slate-400">
									{formatDecimalString(pos.mark)}
								</div>
							</td>
							<td class="py-2 pr-3">
								<div class="flex items-center gap-1">
									<span>{formatDecimalString(pos.stop)}</span>
									{#if pos.be_done}
										<span class="rounded bg-emerald-500/20 px-1 text-[10px] text-emerald-300">BE</span>
									{/if}
								</div>
								<div class="flex items-center gap-1 text-[10px] text-slate-400">
									<span>{formatDecimalString(pos.tp1)}</span>
									{#if pos.tp1_done}
										<span class="rounded bg-sky-500/20 px-1 text-sky-300">TP1</span>
									{/if}
								</div>
							</td>
							<td class="py-2 pr-3 text-slate-400">{formatDuration(pos.age_s)}</td>
							{#if canMutate}
								<td class="py-2 text-right">
									<button
										type="button"
										onclick={() => onClosePosition(pos.id)}
										class="rounded border border-red-500/50 bg-red-950/60 px-2 py-1 text-[11px] font-semibold text-red-200 hover:bg-red-800/70"
									>
										Close
									</button>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>
