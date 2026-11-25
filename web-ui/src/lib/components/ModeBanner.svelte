<script lang="ts">
	import type { AuthScope, EngineMode, TradingMode, Venue } from '$lib/api/types';
	import StatusPill from '$lib/components/ui/StatusPill.svelte';
	import { decimalDirectionIcon, decimalSign, formatDecimalString } from '$lib/format/decimal';
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
		onToggleMenu?: () => void;
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
		onDisconnect,
		onToggleMenu
	}: Props = $props();

	const modeBannerClass = $derived(
		buildMode === 'live'
			? 'live-banner-stripes text-white border-red-400 shadow-sm'
			: buildMode === 'testnet'
				? 'bg-amber-400 text-slate-950 border-amber-300'
				: 'bg-sky-600 text-white border-sky-400'
	);

	const engineTone = $derived(
		engineMode === 'Ready'
			? 'positive'
			: engineMode === 'Paused' || engineMode === 'Syncing' || engineMode === 'Starting'
				? 'warning'
				: 'danger'
	);

	const engineIcon = $derived(
		engineMode === 'Ready' ? '●' : engineMode === 'Paused' ? '❚❚' : '⚠'
	);

	const daySign = $derived(decimalSign(dayPnlPct));
</script>

<header class="sticky top-0 z-40 border-b border-hairline bg-canvas/95 backdrop-blur">
	<div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-3 py-2 lg:px-4">
		<div class="flex flex-wrap items-center gap-2">
			{#if onToggleMenu}
				<button
					type="button"
					onclick={onToggleMenu}
					aria-label="Toggle navigation"
					class="inline-flex items-center justify-center rounded border border-hairline bg-surface px-2 py-1 text-xs text-ink-muted hover:border-hairline-strong hover:text-ink-strong lg:hidden"
				>
					☰
				</button>
			{/if}

			<span
				class={`inline-flex items-center gap-1.5 rounded border px-2.5 py-0.5 font-mono text-[11px] font-extrabold tracking-wider uppercase ${modeBannerClass}`}
			>
				<span>{buildMode.toUpperCase()}</span>
			</span>

			<span
				class="inline-flex items-center gap-1 rounded border border-hairline-strong bg-surface px-2 py-0.5 font-mono text-[11px] font-semibold text-ink-strong uppercase"
			>
				<span>{venue.toUpperCase()}</span>
				{#if venue === 'futures'}
					<span class="text-[10px] font-normal text-warning">3× ISO</span>
				{/if}
			</span>

			<StatusPill label={engineMode.toUpperCase()} icon={engineIcon} tone={engineTone} />
		</div>

		<div class="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-[11px] text-ink tabular-nums">
			<span>
				<span class="text-ink-dim uppercase">Eq</span>
				<strong class="ml-1 text-ink-strong">{formatDecimalString(equity, { decimals: 2 })}</strong>
			</span>
			<span>
				<span class="text-ink-dim uppercase">Day</span>
				<strong
					class={`ml-1 ${
						daySign > 0 ? 'text-positive' : daySign < 0 ? 'text-danger' : 'text-ink-strong'
					}`}
				>
					{decimalDirectionIcon(dayPnlPct)}
					{formatDecimalString(dayPnlPct, { signed: true, suffix: '%' })}
				</strong>
			</span>
			<span>
				<span class="text-ink-dim uppercase">DD</span>
				<strong class="ml-1 text-warning">{formatDecimalString(drawdownPct, { suffix: '%' })}</strong>
			</span>
			<span class="hidden text-ink-muted sm:inline">WS {wsLatencyMs}ms</span>
			<span class="hidden text-ink-muted md:inline">W {weightUsed}/{weightLimit}</span>
			<span class={alertCount > 0 ? 'text-warning' : 'text-ink-dim'} title="Active alerts">
				🔔 {alertCount}
			</span>
		</div>

		<div class="flex items-center gap-2">
			{#if authScope === 'full'}
				<button
					type="button"
					onclick={onToggleControls}
					class={`rounded border px-2.5 py-1 font-mono text-[11px] font-semibold transition ${
						canMutate
							? 'border-warning/60 bg-warning/15 text-warning hover:bg-warning/25'
							: 'border-hairline bg-surface text-ink-muted hover:border-hairline-strong hover:text-ink-strong'
					}`}
				>
					{canMutate ? '🔓 Controls ON' : '🔒 Read-Only'}
				</button>
			{:else}
				<span
					class="rounded border border-hairline bg-surface px-2 py-1 font-mono text-[11px] font-medium text-ink-dim"
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
				class="rounded border border-hairline bg-surface px-2.5 py-1 font-mono text-[11px] text-ink-muted transition hover:border-hairline-strong hover:text-ink-strong"
			>
				Logout
			</button>
		</div>
	</div>
</header>
