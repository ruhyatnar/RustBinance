<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		subtitle?: string;
		actions?: Snippet;
	}

	let { title, subtitle, actions }: Props = $props();
</script>

<div class="flex flex-wrap items-end justify-between gap-3 border-b border-hairline-subtle pb-3">
	<div>
		<h1 class="font-display text-xl font-semibold tracking-[-0.03em] text-ink-strong">
			{title}
		</h1>
		{#if subtitle}
			<p class="mt-0.5 text-xs text-ink-muted">{subtitle}</p>
		{/if}
	</div>

	{#if actions}
		<div class="flex flex-wrap items-center gap-2">
			{@render actions()}
		</div>
	{/if}
</div>
