<script lang="ts">
	interface Props {
		label: string;
		value: string;
		sublabel?: string;
		icon?: string;
		tone?: 'neutral' | 'positive' | 'warning' | 'danger' | 'telemetry';
		progressPct?: number | null;
	}

	let {
		label,
		value,
		sublabel,
		icon,
		tone = 'neutral',
		progressPct = null
	}: Props = $props();

	const valueColor = $derived(
		tone === 'positive'
			? 'text-positive'
			: tone === 'warning'
				? 'text-warning'
				: tone === 'danger'
					? 'text-danger'
					: tone === 'telemetry'
						? 'text-telemetry'
						: 'text-ink-strong'
	);

	const barColor = $derived(
		tone === 'positive'
			? 'bg-positive'
			: tone === 'warning'
				? 'bg-warning'
				: tone === 'danger'
					? 'bg-danger-solid'
					: tone === 'telemetry'
						? 'bg-telemetry'
						: 'bg-accent'
	);

	const clampedPct = $derived(
		progressPct === null ? null : Math.max(0, Math.min(100, Math.round(progressPct)))
	);
</script>

<div class="inner-glow flex flex-col justify-between rounded-lg border border-hairline bg-surface p-4">
	<div class="flex items-center justify-between gap-2">
		<span class="text-[11px] font-medium tracking-[0.08em] text-ink-muted uppercase">
			{label}
		</span>
		{#if icon}
			<span class={`font-mono text-xs font-semibold ${valueColor}`} aria-hidden="true">
				{icon}
			</span>
		{/if}
	</div>

	<div class={`mt-2 font-display text-2xl font-semibold tracking-[-0.03em] tabular-nums ${valueColor}`}>
		{value}
	</div>

	{#if sublabel}
		<div class="mt-1.5 font-mono text-[11px] text-ink-dim tabular-nums">
			{sublabel}
		</div>
	{/if}

	{#if clampedPct !== null}
		<div class="mt-3 h-1 overflow-hidden rounded-full bg-surface-inset">
			<div
				class={`h-full transition-[width] duration-200 ${barColor}`}
				style={`width: ${clampedPct}%`}
			></div>
		</div>
	{/if}
</div>
