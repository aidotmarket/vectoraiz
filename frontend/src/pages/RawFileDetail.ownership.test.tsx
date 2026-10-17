import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import RawFileDetail from './RawFileDetail';
import { rawFilesApi, type RawFile } from '@/lib/api';
import type { ComponentProps, ReactNode } from 'react';
import { toast } from '@/hooks/use-toast';
const { route, navigate } = vi.hoisted(() => ({ route: { id: 'file-a' as string | undefined }, navigate: vi.fn() }));
vi.mock('react-router-dom', () => ({ useParams: () => route, useNavigate: () => navigate, Link: (p: ComponentProps<'a'> & {
        to: string;
    }) => <a href={p.to}>{p.children}</a> }));
vi.mock('@/lib/api', () => ({ rawFilesApi: { getRawFile: vi.fn(), updateRawFile: vi.fn(), deleteRawFile: vi.fn(), downloadRawFile: vi.fn(), getFileObjectUrl: vi.fn() } }));
const record = (id: string) => ({ id, filename: id + '.bin', mime_type: 'application/octet-stream', file_size_bytes: 12, listing_status: null, price_cents: null, file_path: 'synthetic-path', content_hash: 'synthetic-hash', created_at: '2026-01-01', updated_at: '2026-01-01', metadata: { title: 'Title ' + id, description: 'Description ' + id, tags: [id], technical_metadata: { preserved: true } } } satisfies RawFile);
const deferred = <T = RawFile,>() => { let resolve!: (x: T) => void; let reject!: (x: Error) => void; const promise = new Promise<T>((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
// Keep presentation, mode, routing, and every file operation synthetic.
vi.mock('@/hooks/use-toast', () => ({ toast: vi.fn() }));
vi.mock('@/contexts/ModeContext', () => ({ useMode: () => ({ isConnected: true }) }));
vi.mock('@/components/ui/button', () => ({ Button: ({ children, asChild, variant, size, ...props }: ComponentProps<'button'> & {
        asChild?: boolean;
        variant?: string;
        size?: string;
    }) => asChild ? <>{children}</> : <button {...props}>{children}</button> }));
vi.mock('@/components/ui/card', () => {
    const Box = ({ children }: {
        children?: ReactNode;
    }) => <div>{children}</div>;
    return { Card: Box, CardContent: Box, CardDescription: Box, CardHeader: Box, CardTitle: Box };
});
vi.mock('@/components/ui/badge', () => ({ Badge: ({ children }: {
        children?: ReactNode;
    }) => <span>{children}</span> }));
vi.mock('@/components/ui/input', () => ({ Input: (props: ComponentProps<'input'>) => <input {...props}/> }));
vi.mock('@/components/ui/label', () => ({ Label: (props: ComponentProps<'label'>) => <label {...props}/> }));
vi.mock('@/components/ui/skeleton', () => ({ Skeleton: () => <div /> }));
vi.mock('@/components/ui/separator', () => ({ Separator: () => <hr /> }));
beforeEach(() => { vi.resetAllMocks(); route.id = 'file-a'; vi.stubGlobal('fetch', vi.fn(() => { throw Error('Unexpected network'); })); vi.stubGlobal('confirm', vi.fn(() => true)); vi.mocked(rawFilesApi.getRawFile).mockImplementation(async (id) => record(id)); vi.mocked(rawFilesApi.updateRawFile).mockImplementation(async (id, metadata) => ({ ...record(id), metadata })); });
afterEach(() => { expect(fetch).not.toHaveBeenCalled(); expect(rawFilesApi.getFileObjectUrl).not.toHaveBeenCalled(); cleanup(); vi.unstubAllGlobals(); });
it('does not display the previous file when the newly selected file read fails', async () => {
    const view = render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    vi.mocked(rawFilesApi.getRawFile).mockRejectedValueOnce(Error('Synthetic B read failure'));
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await act(async () => { });
    await waitFor(() => expect(rawFilesApi.getRawFile).toHaveBeenCalledWith('file-b'));
    expect(screen.queryByRole('heading', { name: 'file-a.bin' })).toBeNull();
});
it('cannot save old file metadata into a new route whose read failed', async () => {
    const view = render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    vi.mocked(rawFilesApi.getRawFile).mockRejectedValueOnce(Error('Synthetic B read failure'));
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await act(async () => { });
    const save = screen.queryByRole('button', { name: 'Save Metadata' });
    if (save) {
        fireEvent.click(save);
        await act(async () => { });
    }
    expect(rawFilesApi.updateRawFile).not.toHaveBeenCalled();
});
it('ignores an older file read that completes after the current file', async () => {
    const old = deferred();
    vi.mocked(rawFilesApi.getRawFile).mockReturnValueOnce(old.promise);
    const view = render(<RawFileDetail />);
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-b.bin' });
    await act(async () => old.resolve(record('file-a')));
    expect(screen.queryByRole('heading', { name: 'file-a.bin' })).toBeNull();
    expect(screen.getByLabelText('Title').getAttribute('value')).toBe('Title file-b');
});
it('does not replace the current file when an earlier save completes', async () => {
    const old = deferred();
    vi.mocked(rawFilesApi.updateRawFile).mockReturnValueOnce(old.promise);
    const view = render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.click(screen.getByRole('button', { name: 'Save Metadata' }));
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-b.bin' });
    await act(async () => old.resolve(record('file-a')));
    expect(screen.queryByRole('heading', { name: 'file-a.bin' })).toBeNull();
    expect(screen.getByRole('heading', { name: 'file-b.bin' })).toBeTruthy();
});
it('does not navigate away from the current file when an earlier delete completes', async () => {
    const old = deferred<void>();
    vi.mocked(rawFilesApi.deleteRawFile).mockReturnValueOnce(old.promise);
    const view = render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-b.bin' });
    await act(async () => old.resolve(undefined));
    expect(navigate).not.toHaveBeenCalled();
    expect(rawFilesApi.deleteRawFile).toHaveBeenCalledExactlyOnceWith('file-a');
});
it('still saves the current file and preserves hidden metadata', async () => {
    render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Edited current title' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save Metadata' }));
    await act(async () => { });
    expect(rawFilesApi.updateRawFile).toHaveBeenCalledExactlyOnceWith('file-a', { ...record('file-a').metadata, title: 'Edited current title' });
    expect(rawFilesApi.deleteRawFile).not.toHaveBeenCalled();
    expect(rawFilesApi.downloadRawFile).not.toHaveBeenCalled();
});
const settle = async (request: ReturnType<typeof deferred<RawFile>>, outcome: 'success' | 'failure', id = 'file-a') => {
    await act(async () => {
        if (outcome === 'success')
            request.resolve(record(id));
        else
            request.reject(Error('Synthetic failure'));
    });
};
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const title = () => screen.getByLabelText('Title') as HTMLInputElement;
it.each(['success', 'failure'] as const)('keeps pending B isolated from old A read %s/finally', async (outcome) => {
    const old = deferred(), current = deferred();
    vi.mocked(rawFilesApi.getRawFile).mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const view = render(<RawFileDetail />);
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await settle(old, outcome);
    expect(screen.queryByRole('button', { name: 'Save Metadata' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Create Listing' })).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(toast).not.toHaveBeenCalled();
    await settle(current, 'success', 'file-b');
    expect(title().value).toBe('Title file-b');
});
it('ignores old read failure/finally after B loads', async () => {
    const old = deferred();
    vi.mocked(rawFilesApi.getRawFile).mockReturnValueOnce(old.promise);
    const view = render(<RawFileDetail />);
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-b.bin' });
    await settle(old, 'failure');
    expect(title().value).toBe('Title file-b');
    expect(button('Save Metadata').disabled).toBe(false);
    expect(toast).not.toHaveBeenCalled();
});
it.each(['success', 'failure'] as const)('retires first A read across A -> B -> A (%s)', async (outcome) => {
    const first = deferred(), latest = deferred();
    vi.mocked(rawFilesApi.getRawFile).mockReturnValueOnce(first.promise)
        .mockResolvedValueOnce(record('file-b')).mockReturnValueOnce(latest.promise);
    const view = render(<RawFileDetail />);
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-b.bin' });
    route.id = 'file-a';
    view.rerender(<RawFileDetail />);
    await settle(first, outcome);
    expect(screen.queryByRole('button', { name: 'Save Metadata' })).toBeNull();
    expect(toast).not.toHaveBeenCalled();
    await act(async () => latest.resolve({ ...record('file-a'), metadata: { title: 'New visit' } }));
    expect(title().value).toBe('New visit');
});
it.each(['save', 'delete'] as const)('ignores retired %s success and failure without changing current busy state', async (operation) => {
    for (const outcome of ['success', 'failure'] as const) {
        const old = deferred(), current = deferred();
        if (operation === 'save') {
            vi.mocked(rawFilesApi.updateRawFile).mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
        } else {
            vi.mocked(rawFilesApi.deleteRawFile).mockReturnValueOnce(old.promise.then(() => undefined)).mockReturnValueOnce(current.promise.then(() => undefined));
        }
        const view = render(<RawFileDetail />);
        await screen.findByRole('heading', { name: 'file-a.bin' });
        const name = operation === 'save' ? 'Save Metadata' : 'Delete';
        fireEvent.click(button(name));
        route.id = 'file-b';
        view.rerender(<RawFileDetail />);
        await screen.findByRole('heading', { name: 'file-b.bin' });
        expect(button(name).disabled).toBe(false);
        fireEvent.change(title(), { target: { value: 'B draft' } });
        fireEvent.click(button(name));
        await settle(old, outcome);
        expect(button(name).disabled).toBe(true);
        expect(title().value).toBe('B draft');
        expect(toast).not.toHaveBeenCalled();
        expect(navigate).not.toHaveBeenCalled();
        view.unmount();
        await settle(current, 'failure');
        expect(toast).not.toHaveBeenCalled();
        expect(navigate).not.toHaveBeenCalled();
        route.id = 'file-a';
    }
});
it.each(['read', 'save', 'delete'] as const)('suppresses %s effects after unmount', async (operation) => {
    for (const outcome of ['success', 'failure'] as const) {
        const pending = deferred();
        if (operation === 'read')
            vi.mocked(rawFilesApi.getRawFile).mockReturnValueOnce(pending.promise);
        else if (operation === 'save')
            vi.mocked(rawFilesApi.updateRawFile).mockReturnValueOnce(pending.promise);
        else
            vi.mocked(rawFilesApi.deleteRawFile).mockReturnValueOnce(pending.promise.then(() => undefined));
        const view = render(<RawFileDetail />);
        if (operation !== 'read') {
            await screen.findByRole('heading', { name: 'file-a.bin' });
            fireEvent.click(button(operation === 'save' ? 'Save Metadata' : 'Delete'));
        }
        view.unmount();
        await settle(pending, outcome);
        expect(toast).not.toHaveBeenCalled();
        expect(navigate).not.toHaveBeenCalled();
    }
});
it('retries only the selected file explicitly, including repeated failures', async () => {
    const view = render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    vi.mocked(rawFilesApi.getRawFile).mockRejectedValueOnce(Error('B failed')).mockRejectedValueOnce(Error('B failed again'));
    route.id = 'file-b';
    view.rerender(<RawFileDetail />);
    await screen.findByRole('alert');
    expect(screen.getByRole('link', { name: 'Back to Datasets' }).getAttribute('href')).toBe('/datasets');
    expect(rawFilesApi.getRawFile).toHaveBeenCalledTimes(2);
    fireEvent.click(button('Retry'));
    await screen.findByRole('alert');
    expect(rawFilesApi.getRawFile).toHaveBeenCalledTimes(3);
    expect(screen.queryByText('File not found.')).toBeNull();
    fireEvent.click(button('Retry'));
    await screen.findByRole('heading', { name: 'file-b.bin' });
    expect(vi.mocked(rawFilesApi.getRawFile).mock.calls.map(([id]) => id)).toEqual(['file-a', 'file-b', 'file-b', 'file-b']);
    fireEvent.click(button('Save Metadata'));
    await act(async () => { });
    expect(rawFilesApi.updateRawFile).toHaveBeenCalledWith('file-b', record('file-b').metadata);
});
it('offers Back without a read or indefinite loading for a missing id', async () => {
    route.id = undefined;
    render(<RawFileDetail />);
    expect(await screen.findByText('No file selected.')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'Back to Datasets' })).toBeTruthy();
    expect(rawFilesApi.getRawFile).not.toHaveBeenCalled();
});
it('rejects a response for a different record', async () => {
    vi.mocked(rawFilesApi.getRawFile).mockResolvedValueOnce(record('file-b'));
    render(<RawFileDetail />);
    await screen.findByRole('alert');
    expect(screen.queryByRole('button', { name: 'Save Metadata' })).toBeNull();
    expect(screen.queryByRole('heading', { name: 'file-b.bin' })).toBeNull();
});
it('preserves edits typed during a save and submits them on the next save', async () => {
    const pending = deferred();
    vi.mocked(rawFilesApi.updateRawFile).mockReturnValueOnce(pending.promise);
    render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.change(title(), { target: { value: 'Submitted' } });
    fireEvent.click(button('Save Metadata'));
    fireEvent.change(title(), { target: { value: 'Still typing' } });
    fireEvent.change(screen.getByLabelText('Tags (comma-separated)'), { target: { value: ' one, , two ' } });
    await settle(pending, 'success');
    expect(title().value).toBe('Still typing');
    expect(button('Save Metadata').disabled).toBe(false);
    fireEvent.click(button('Save Metadata'));
    await act(async () => { });
    expect(rawFilesApi.updateRawFile).toHaveBeenLastCalledWith('file-a', { ...record('file-a').metadata, title: 'Still typing', tags: ['one', 'two'] });
});
it('preserves current delete confirmation, cancel, success, and failure behavior', async () => {
    render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    vi.mocked(confirm).mockReturnValueOnce(false);
    fireEvent.click(button('Delete'));
    expect(rawFilesApi.deleteRawFile).not.toHaveBeenCalled();
    expect(confirm).toHaveBeenCalledWith('Are you sure you want to delete this file?');
    vi.mocked(rawFilesApi.deleteRawFile).mockRejectedValueOnce(Error('Failed'));
    fireEvent.click(button('Delete'));
    await act(async () => { });
    expect(button('Delete').disabled).toBe(false);
    expect(navigate).not.toHaveBeenCalled();
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ description: 'Failed to delete file' }));
    fireEvent.click(button('Delete'));
    await act(async () => { });
    expect(rawFilesApi.deleteRawFile).toHaveBeenLastCalledWith('file-a');
    expect(navigate).toHaveBeenCalledExactlyOnceWith('/datasets');
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ title: 'Deleted' }));
});
it('keeps current listing, download, and vectorize actions useful', async () => {
    render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.click(button('Create Listing'));
    const meta = record('file-a').metadata;
    expect(navigate).toHaveBeenCalledExactlyOnceWith('/ai-market', { state: { createListing: { rawFileId: 'file-a', title: meta.title, description: meta.description, tags: meta.tags } } });
    fireEvent.click(button('Download'));
    await act(async () => { });
    expect(rawFilesApi.downloadRawFile).toHaveBeenCalledExactlyOnceWith(record('file-a'));
    fireEvent.click(button('Vectorize This File'));
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ title: 'Coming Soon' }));
});
it.each(['save', 'delete'] as const)('retires first A %s completion across A -> B -> A', async (operation) => {
    for (const outcome of ['success', 'failure'] as const) {
        const pending = deferred();
        if (operation === 'save')
            vi.mocked(rawFilesApi.updateRawFile).mockReturnValueOnce(pending.promise);
        else
            vi.mocked(rawFilesApi.deleteRawFile).mockReturnValueOnce(pending.promise.then(() => undefined));
        const view = render(<RawFileDetail />);
        await screen.findByRole('heading', { name: 'file-a.bin' });
        fireEvent.click(button(operation === 'save' ? 'Save Metadata' : 'Delete'));
        route.id = 'file-b';
        view.rerender(<RawFileDetail />);
        await screen.findByRole('heading', { name: 'file-b.bin' });
        route.id = 'file-a';
        view.rerender(<RawFileDetail />);
        await screen.findByRole('heading', { name: 'file-a.bin' });
        fireEvent.change(title(), { target: { value: 'Latest A draft' } });
        await settle(pending, outcome);
        expect(title().value).toBe('Latest A draft');
        expect(button('Save Metadata').disabled).toBe(false);
        expect(button('Delete').disabled).toBe(false);
        expect(toast).not.toHaveBeenCalled();
        expect(navigate).not.toHaveBeenCalled();
        view.unmount();
    }
});
it('keeps current save errors recoverable and retains the draft', async () => {
    vi.mocked(rawFilesApi.updateRawFile).mockRejectedValueOnce(Error('Synthetic save failure'));
    render(<RawFileDetail />);
    await screen.findByRole('heading', { name: 'file-a.bin' });
    fireEvent.change(title(), { target: { value: 'Draft' } });
    fireEvent.click(button('Save Metadata'));
    await act(async () => { });
    expect(button('Save Metadata').disabled).toBe(false);
    expect(title().value).toBe('Draft');
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ description: 'Failed to save metadata' }));
    fireEvent.click(button('Save Metadata'));
    await act(async () => { });
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ title: 'Saved' }));
});
