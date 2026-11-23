<script lang="ts">
	import type { AuthScope, EngineMode, TradingMode, Venue } from '$lib/api/types';
	import { decimalDirectionIcon, formatDecimalString } from '$lib/format/decimal';
	import HoldToConfirm from './HoldToConfirm.svelte';

	interface Props {
		buildMode: TradingMode;
		venue: Venue;
		engineMode: EngineMode;
		equity: string;
		dayPnlPct: string;
		drawdownPct: string;
		wsLatencyMs: number;
		weightUsed: number;
		weightLimit: number;
		alertCount: number;
		authScope: AuthScope;
		canMutate: boolean;
		onPanic: () => void | Promise<void>;
		onToggleControls: () => void;
		onDisconnect: () => void;
	}

	let {
		buildMode,
		venue,
		engineMode,
		equity,
		dayPnlPct,
		drawdownPct,
		wsLatencyMs,
		weightUsed,
		weightLimit,
		alertCount,
		authScope,
		canMutate,
		onPanic,
		onToggleControls,
		onDisconnect
	}: Props = $props();

	const modeBannerClass = $derived(
		buildMode === 'live'
			? 'live-banner-stripes text-white border-red-400'
			: buildMode === 'testnet'
				? 'bg-amber-400 text-slate-950 border-amber-300'
				: 'bg-sky-600 text-white border-sky-400'
	);

	const enginePillClass = $derived(
		engineMode === 'Ready'
			? 'bg-emerald-500/20 text-emerald-300 border-emerald-500/50'
			: engineMode === 'Paused' || engineMode === 'Syncing' || engineMode === 'Starting'
				? 'bg-amber-500/20 text-amber-300 border-amber-500/50'
				: 'bg-red-500/20 text-red-200 border-red-500/60'
	);

	const engineIcon = $derived(
		engineMode === 'Ready' ? '●' : engineMode === 'Paused' ? '❚❚' : '⚠'
	);
</script>

<header class="sticky top-0 z-30 border-b border-slate-800 bg-slate-950/95 backdrop-blur">
	<div class="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
		<div class="flex flex-wrap items-center gap-2">
			<span
				class={`inline-flex items-center gap-1.5 rounded border px-2.5 py-0.5 text-xs font-extrabold tracking-wider uppercase ${modeBannerClass}`}
			>
				<span>{buildMode.toUpperCase()}</span>
			</span>

			<span
				class="inline-flex items-center rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-xs font-semibold text-slate-200 uppercase"
			>
				{venue.toUpperCase()}
			</span>

			<span
				class={`inline-flex items-center gap-1 rounded border px-2 py-0.5 text-xs font-bold uppercase ${enginePillClass}`}
			>
				<span>{engineIcon}</span>
				<span>{engineMode.toUpperCase()}</span>
			</span>
		</div>

		<div class="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-xs text-slate-200">
			<span>
				<span class="text-slate-400">Eq</span>
				<strong class="ml-1 text-white">{formatDecimalString(equity, { decimals: 2 })}</strong>
			</span>
			<span>
				<span class="text-slate-400">Day</span>
				<strong class="ml-1">
					{decimalDirectionIcon(dayPnlPct)}
					{formatDecimalString(dayPnlPct, { signed: true, suffix: '%' })}
				</strong>
			</span>
			<span>
				<span class="text-slate-400">DD</span>
				<strong class="ml-1">{formatDecimalString(drawdownPct, { suffix: '%' })}</strong>
			</span>
			<span class="hidden sm:inline text-slate-300">WS {wsLatencyMs}ms</span>
			<span class="hidden md:inline text-slate-300">W {weightUsed}/{weightLimit}</span>
			<span class="text-amber-300" title="Active alerts">🔔 {alertCount}</span>
		</div>

		<div class="flex items-center gap-2">
			{#if authScope === 'full'}
				<button
					type="button"
					onclick={onToggleControls}
					class={`rounded border px-2.5 py-1 text-xs font-semibold transition ${
						canMutate
							? 'border-amber-500/60 bg-amber-500/20 text-amber-200 hover:bg-amber-500/30'
							: 'border-slate-700 bg-slate-900 text-slate-300 hover:border-slate-500'
					}`}
				>
					{canMutate ? '🔓 Controls ON' : '🔒 Read-Only'}
				</button>
			{:else}
				<span
					class="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs font-medium text-slate-400"
				>
					🔒 Read-Only Token
				</span>
			{/if}

			{#if canMutate}
				<HoldToConfirm label="PANIC" holdMs={1500} onConfirm={onPanic} />
			{/if}

			<button
				type="button"
				onclick={onDisconnect}
				class="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-slate-300 hover:border-slate-500 hover:text-white"
			>
				Logout
			</button>
		</div>
	</div>
</header>
