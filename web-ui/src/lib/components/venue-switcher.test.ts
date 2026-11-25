import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VenuesSummary } from '$lib/api/types';
import VenueSwitchDialog from './VenueSwitchDialog.svelte';

describe('VenueSwitcher & VenueSwitchDialog contract (§11.6)', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('represents Spot Draining + Futures Active cleanly in VenuesSummary', () => {
		const summary: VenuesSummary = {
			active: 'futures',
			spot: 'Draining',
			futures: 'Active',
			last_switch_ms: 1_700_000_000_000,
			switches_today: 1
		};
		expect(summary.active).toBe('futures');
		expect(summary.spot).toBe('Draining');
		expect(summary.futures).toBe('Active');
	});

	it('renders HoldToConfirm (1.5s) when flatten policy is selected and blocks instant click', async () => {
		const onConfirm = vi.fn();
		render(VenueSwitchDialog, {
			props: {
				open: true,
				currentVenue: 'spot',
				targetVenue: 'futures',
				engineMode: 'Ready',
				openPositionsCount: 2,
				openPnlUsd: '+14.20',
				onConfirm
			}
		});

		const flattenRadio = screen.getByRole('radio', { name: /Tutup semua sekarang/i });
		await fireEvent.click(flattenRadio);

		const holdBtn = screen.getByRole('button', { name: /Flatten & Alihkan \(hold 1\.5s\)/i });
		await fireEvent.click(holdBtn);
		expect(onConfirm).not.toHaveBeenCalled();

		await fireEvent.pointerDown(holdBtn);
		await vi.advanceTimersByTimeAsync(1550);
		expect(onConfirm).toHaveBeenCalledTimes(1);
		expect(onConfirm).toHaveBeenCalledWith('flatten');
	});

	it('disables confirmation and shows blocked reason when engineMode is Halted', () => {
		render(VenueSwitchDialog, {
			props: {
				open: true,
				currentVenue: 'spot',
				targetVenue: 'futures',
				engineMode: 'Halted',
				openPositionsCount: 1,
				onConfirm: vi.fn()
			}
		});

		expect(screen.getByTestId('switch-blocked-reason')).toHaveTextContent(/Halted/i);
		expect(screen.getByTestId('confirm-switch-btn')).toBeDisabled();
	});
});
