import { cleanup, render, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import type { RawFile } from '@/lib/api';

const h = vi.hoisted(() => ({
  mode: { isStandalone: false, isConnected: true, isLoading: false, hasFeature: (_name: string): boolean => true },
  cp: { connectionStatus: 'connected' as 'connected' | 'connecting' | 'disconnected' },
  backend: 'connected',
  channel: 'direct',
  deny: vi.fn(() => { throw Error('Unexpected network or file operation'); }),
  read: vi.fn<(id: string) => Promise<RawFile>>(),
}));
vi.mock('@/contexts/ModeContext', () => ({ useMode: () => h.mode }));
vi.mock('@/contexts/CoPilotContext', () => ({ useCoPilot: () => h.cp }));
vi.mock('@/contexts/BrandContext', () => ({ useBrand: () => ({ name: 'Fixture Brand', externalUrl: '#', logoPath: 'data:,', sidebarLogoAlt: 'Fixture' }) }));
vi.mock('@/contexts/AuthContext', () => ({ useAuth: () => ({ logout: h.deny }) }));
vi.mock('@/hooks/useChannel', () => ({ useChannel: () => h.channel }));
vi.mock('@/hooks/useApi', () => ({ useBackendConnection: () => ({ status: h.backend }) }));
vi.mock('@/hooks/use-toast', () => ({ toast: h.deny }));
vi.mock('@/components/notifications/NotificationBell', () => ({ default: () => null }));
vi.mock('@/components/UploadIndicator', () => ({ default: () => null }));
// Keep tooltip text mounted so every status description is asserted without timers.
vi.mock('@/components/ui/tooltip', () => {
  const Box = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return { Tooltip: Box, TooltipTrigger: Box, TooltipContent: Box };
});
vi.mock('@/lib/api', () => ({ rawFilesApi: {
  getRawFile: (id: string) => h.read(id),
  getFileObjectUrl: h.deny, downloadRawFile: h.deny, updateRawFile: h.deny, deleteRawFile: h.deny,
} }));

import Sidebar from './Sidebar';
import TopBar from './TopBar';
import RawFileDetail from '@/pages/RawFileDetail';

const file: RawFile = {
  id: 'owned-raw', filename: 'owned.bin', file_size_bytes: 128, mime_type: 'application/octet-stream',
  listing_status: null, price_cents: null, file_path: 'fixture', content_hash: 'fixture',
  created_at: '2026-01-01', updated_at: '2026-01-01',
  metadata: { title: 'Owned fixture', description: 'Fixture', tags: ['owned'] },
};
const sidebar = (collapsed = false) => <MemoryRouter><Sidebar collapsed={collapsed} onToggle={h.deny} /></MemoryRouter>;
const topbar = () => <MemoryRouter><TopBar /></MemoryRouter>;
const raw = () => <MemoryRouter initialEntries={['/raw-files/owned-raw']}><Routes><Route path="/raw-files/:id" element={<RawFileDetail />} /></Routes></MemoryRouter>;
const row = (view: ReturnType<typeof render>, label: string) => view.getByText(label).closest('li')!;
const passed = (element: Element) => element.querySelector('svg')?.classList.contains('text-green-500');

beforeEach(() => {
  Object.assign(h.mode, { isStandalone: false, isConnected: true, isLoading: false, hasFeature: (_name: string): boolean => true });
  h.cp.connectionStatus = 'connected';
  h.backend = 'connected';
  h.channel = 'direct';
  h.deny.mockClear();
  h.read.mockReset().mockResolvedValue(file);
  vi.stubGlobal('fetch', h.deny);
  vi.stubGlobal('XMLHttpRequest', h.deny);
  vi.stubGlobal('WebSocket', h.deny);
  vi.stubGlobal('crypto', { randomUUID: () => 'owned-status' });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  expect(h.deny).not.toHaveBeenCalled();
});

it('updates the sidebar from checking to confirmed standalone to configured mode', () => {
  Object.assign(h.mode, { isLoading: true, isStandalone: true, isConnected: false });
  const view = render(sidebar());
  expect(view.getByText('Checking mode...')).toBeTruthy();
  expect(view.queryByText('Standalone Mode')).toBeNull();
  h.mode.isLoading = false;
  view.rerender(sidebar());
  expect(view.getByText('Standalone Mode')).toBeTruthy();
  Object.assign(h.mode, { isStandalone: false, isConnected: true });
  view.rerender(sidebar());
  expect(view.getByText('Connected mode configured')).toBeTruthy();
  expect(view.container.textContent).not.toContain('Connected to ai.market');
  h.mode.isLoading = true;
  view.rerender(sidebar());
  expect(view.getByText('Checking mode...')).toBeTruthy();
  expect(view.queryByText('Connected mode configured')).toBeNull();
});

it.each(['direct', 'marketplace', 'aim-data'])('preserves navigation and feature gating in the %s channel', (channel) => {
  h.channel = channel;
  const view = render(sidebar());
  const paths = () => Array.from(view.container.querySelectorAll('nav a, ul a')).map(a => a.getAttribute('href'));
  const before = paths();
  for (const [name, href] of [['Billing', '/billing'], ['Earnings', '/earnings'], ['ai.market', '/ai-market']]) {
    expect(view.getByRole('link', { name }).getAttribute('href')).toBe(href);
  }
  h.mode.isLoading = true;
  view.rerender(sidebar());
  expect(paths()).toEqual(before);
  h.mode.hasFeature = () => false;
  view.rerender(sidebar());
  expect(view.queryByRole('link', { name: 'Earnings' })).toBeNull();
  expect(view.getByRole('link', { name: 'Billing' })).toBeTruthy();
  view.rerender(sidebar(true));
  expect(view.queryByText('Checking mode...')).toBeNull();
  expect(view.getByAltText('Fixture')).toBeTruthy();
});

it.each(['connected', 'disconnected', 'checking'])('keeps local assistant states distinct from local backend %s and remote health', (backend) => {
  h.backend = backend;
  const view = render(topbar());
  for (const status of ['connected', 'connecting', 'disconnected'] as const) {
    h.cp.connectionStatus = status;
    view.rerender(topbar());
    expect(view.getByText(`Local assistant ${status}${status === 'connecting' ? '...' : ''}`)).toBeTruthy();
    expect(view.container.textContent).toContain(`Local backend: ${backend === 'connected' ? 'OK' : backend}.`);
    expect(view.container.textContent).toContain('Remote ai.market health is not checked.');
    expect(view.container.textContent).not.toMatch(/(?:Connected|Not connected|Connecting) to ai\.market/);
  }
});

it('does not infer assistant availability from confirmed standalone mode', () => {
  Object.assign(h.mode, { isStandalone: true, isConnected: false });
  h.cp.connectionStatus = 'disconnected';
  const view = render(topbar());
  expect(view.getByText('Local assistant disconnected')).toBeTruthy();
  expect(view.container.textContent).not.toMatch(/unavailable|outage|setup required/i);
});

it.each([true, false])('keeps raw readiness pending while mode loads (configured=%s), without changing controls or refetching', async (connected) => {
  Object.assign(h.mode, { isLoading: true, isConnected: connected, isStandalone: !connected });
  const view = render(raw());
  await waitFor(() => expect(view.getByText('Listing Readiness')).toBeTruthy());
  expect(passed(row(view, 'Checking mode configuration...'))).toBe(false);
  const controls = () => Array.from(view.container.querySelectorAll('button')).map(b => [b.textContent, b.disabled]);
  const before = controls();
  h.mode.isLoading = false;
  view.rerender(raw());
  expect(passed(row(view, 'Connected mode configured'))).toBe(connected);
  expect(controls()).toEqual(before);
  expect(passed(row(view, 'File registered'))).toBe(true);
  expect(passed(row(view, 'Metadata complete (title, description, tags)'))).toBe(true);
  expect(passed(row(view, 'Price set (if paid listing)'))).toBe(false);
  expect(view.container.textContent).not.toMatch(/trust channel active|Connected to ai\.market/);
  expect(h.read).toHaveBeenCalledExactlyOnceWith('owned-raw');
  for (const name of ['Download', 'Delete', 'Save Metadata', 'Create Listing', 'Vectorize This File']) {
    expect((view.getByRole('button', { name }) as HTMLButtonElement).disabled).toBe(false);
  }
  h.mode.isLoading = true;
  view.rerender(raw());
  expect(passed(row(view, 'Checking mode configuration...'))).toBe(false);
  expect(controls()).toEqual(before);
});
