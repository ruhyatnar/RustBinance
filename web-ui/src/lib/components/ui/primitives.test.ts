import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import KpiCard from './KpiCard.svelte';
import PageHeader from './PageHeader.svelte';
import StatusPill from './StatusPill.svelte';

describe('Romer UI primitives', () => {
	it('StatusPill renders icon and text label together for colorblind accessibility', () => {
		render(StatusPill, { props: { label: 'READY', icon: '●', tone: 'positive' } });
		expect(screen.getByText('●')).toBeInTheDocument();
		expect(screen.getByText('READY')).toBeInTheDocument();
	});

	it('KpiCard renders label, tabular value, optional icon/sublabel, and progress bar', () => {
		const { container } = render(KpiCard, {
			props: {
				label: 'Portfolio Heat',
				value: '0.90%',
				sublabel: 'Max 2.00% (Account)',
				icon: '▲',
				tone: 'telemetry',
				progressPct: 45
			}
		});
		expect(screen.getByText('Portfolio Heat')).toBeInTheDocument();
		expect(screen.getByText('0.90%')).toBeInTheDocument();
		expect(screen.getByText('Max 2.00% (Account)')).toBeInTheDocument();
		expect(screen.getByText('▲')).toBeInTheDocument();
		expect(container.querySelector('[style*="width: 45%"]')).not.toBeNull();
	});

	it('PageHeader renders title and subtitle', () => {
		render(PageHeader, { props: { title: 'Cockpit', subtitle: 'Live telemetry' } });
		expect(screen.getByRole('heading', { name: 'Cockpit' })).toBeInTheDocument();
		expect(screen.getByText('Live telemetry')).toBeInTheDocument();
	});
});
