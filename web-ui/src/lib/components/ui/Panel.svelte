<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title?: string;
		subtitle?: string;
		dotGrid?: boolean;
		padding?: 'none' | 'sm' | 'md';
		badge?: Snippet;
		actions?: Snippet;
		children: Snippet;
	}

	let {
		title,
		subtitle,
		dotGrid = false,
		padding = 'md',
		badge,
		actions,
		children
	}: Props = $props();

	const bodyPadding = $derived(
		padding === 'none' ? '' : padding === 'sm' ? 'p-3' : 'p-4'
	);
</script>

<section
	class={`inner-glow overflow-hidden rounded-lg border border-hairline bg-surface ${
		dotGrid ? 'dot-grid' : ''
	}`}
>
	{#if title || badge || actions}
		<div
			class="flex flex-wrap items-center justify-between gap-2 border-b border-hairline-subtle bg-surface/90 px-4 py-2.5"
		>
			<div class="flex items-center gap-2.5">
				{#if title}
					<h2 class="font-display text-[11px] font-semibold tracking-[0.08em] text-ink-muted uppercase">
						{title}
					</h2>
				{/if}
				{#if subtitle}
					<span class="text-[11px] text-ink-dim">{subtitle}</span>
				{/if}
				{#if badge}
					{@render badge()}
				{/if}
			</div>

			{#if actions}
				<div class="flex items-center gap-2">
					{@render actions()}
				</div>
			{/if}
		</div>
	{/if}

	<div class={bodyPadding}>
		{@render children()}
	</div>
</section>
