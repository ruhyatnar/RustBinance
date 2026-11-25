<script lang="ts">
	interface Props {
		label: string;
		icon?: string;
		tone?: 'neutral' | 'accent' | 'telemetry' | 'positive' | 'warning' | 'danger';
	}

	let { label, icon, tone = 'neutral' }: Props = $props();

	const toneClass = $derived(
		tone === 'accent'
			? 'border-accent/40 bg-accent/15 text-accent-soft'
			: tone === 'telemetry'
				? 'border-telemetry/40 bg-telemetry/10 text-telemetry'
				: tone === 'positive'
					? 'border-positive/40 bg-positive/10 text-positive'
					: tone === 'warning'
						? 'border-warning/40 bg-warning/10 text-warning'
						: tone === 'danger'
							? 'border-danger-solid/60 bg-danger-solid/15 text-danger'
							: 'border-hairline-strong bg-surface-inset text-ink-muted'
	);
</script>

<span
	class={`inline-flex items-center gap-1 rounded border px-2 py-0.5 font-mono text-[11px] font-semibold tracking-wider uppercase ${toneClass}`}
>
	{#if icon}
		<span aria-hidden="true">{icon}</span>
	{/if}
	<span>{label}</span>
</span>
