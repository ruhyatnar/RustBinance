import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { VenuesSummary } from '$lib/api/types';
import SidebarNav from '$lib/components/SidebarNav.svelte';

const venues: VenuesSummary = {
	active: 'spot',
	spot: 'Active',
	futures: 'Disabled',
	last_switch_ms: 0,
	switches_today: 0
};

describe('SidebarNav (App Shell)', () => {
	it('renders all 9 grouped routes and marks the active path with aria-current="page"', () => {
		render(SidebarNav, {
			props: {
				currentPath: '/risk',
				venues,
				canMutate: false,
				drainingOpenPositions: 0,
				open: false,
				onClose: vi.fn(),
				onRequestSwitch: vi.fn()
			}
		});

		expect(screen.getByText('Trading')).toBeInTheDocument();
		expect(screen.getByText('Analytics')).toBeInTheDocument();
		expect(screen.getAllByText('System').length).toBeGreaterThanOrEqual(2);

		const expectedLabels = [
			'Cockpit',
			'Positions',
			'Signals',
			'Market',
			'Risk',
			'Performance',
			'Logs',
			'Settings',
			'System'
		];
		for (const label of expectedLabels) {
			expect(screen.getByRole('link', { name: new RegExp(label, 'i') })).toBeInTheDocument();
		}

		expect(screen.getByRole('link', { name: /Risk/i })).toHaveAttribute('aria-current', 'page');
		expect(screen.queryByTestId('venue-switcher')).not.toBeInTheDocument();
	});

	it('closes mobile drawer on link click and shows VenueSwitcher when canMutate is true', async () => {
		const onClose = vi.fn();
		render(SidebarNav, {
			props: {
				currentPath: '/',
				venues,
				canMutate: true,
				drainingOpenPositions: 0,
				open: true,
				onClose,
				onRequestSwitch: vi.fn()
			}
		});

		expect(screen.getAllByTestId('venue-switcher').length).toBeGreaterThanOrEqual(1);
		const links = screen.getAllByRole('link', { name: /Positions/i });
		await fireEvent.click(links[links.length - 1]);
		expect(onClose).toHaveBeenCalledTimes(1);
	});
});
