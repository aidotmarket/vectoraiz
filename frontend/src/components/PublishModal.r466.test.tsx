import React from 'react';
import {render,screen,fireEvent,cleanup,act} from '@testing-library/react';
import {describe,it,expect,vi,beforeEach,afterEach} from 'vitest';
const h=vi.hoisted(()=>({deny:vi.fn(()=>{throw new Error('Unexpected transport/write');}),scan:vi.fn()}));
vi.mock('@/lib/api',()=>({piiApi:{getScan:h.scan},getApiUrl:h.deny}));
vi.mock('@/contexts/AuthContext',()=>({useAuth:()=>({apiKey:null})}));
vi.mock('@/hooks/use-toast',()=>({toast:h.deny}));
import type { Dataset } from '@/types/mockDatasets';
import PublishModal from './PublishModal';

const dataset: Dataset = {
  id: 'synthetic-weather', name: 'rainfall.csv', rows: 12, type: 'csv', status: 'ready',
  columns: 2, size: '1 KB', sizeBytes: 1024, processingTime: 0,
  createdAt: new Date('2026-10-01'), modifiedAt: new Date('2026-10-01'),
};

beforeEach(()=>{
  h.scan.mockResolvedValue({column_results:[]});
  vi.stubGlobal('fetch',h.deny); vi.stubGlobal('XMLHttpRequest',h.deny); vi.stubGlobal('WebSocket',h.deny);
  vi.spyOn(window,'open').mockImplementation(h.deny);
  for(const name of ['setItem','removeItem','clear'] as const) vi.spyOn(Storage.prototype,name).mockImplementation(h.deny);
});
afterEach(()=>{
  cleanup(); expect(h.deny).not.toHaveBeenCalled();
  vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();
});
async function openMetadata(){
  render(<PublishModal open onOpenChange={h.deny} dataset={dataset} onPublishSuccess={h.deny}/>);
  await act(async()=>{});
  fireEvent.click(screen.getByLabelText('I confirm I have reviewed the privacy implications'));
  fireEvent.click(screen.getByText('Next'));
  expect(screen.getByLabelText('Title')).toHaveValue('rainfall');
  expect(screen.getByLabelText('Description')).toHaveValue('Dataset containing 12 rows with structured data.');
  fireEvent.change(screen.getByLabelText('Title'),{target:{value:'Synthetic rainfall observations'}});
  fireEvent.change(screen.getByLabelText('Description'),{target:{value:'Twelve daily rainfall observations from a synthetic weather station.'}});
  fireEvent.change(screen.getByPlaceholderText('Add a tag...'),{target:{value:'rainfall'}});
  fireEvent.keyDown(screen.getByPlaceholderText('Add a tag...'),{key:'Enter'});
}
describe('Listing metadata truth and manual editing',()=>{
  it('preserves seller weather claims instead of replacing them with unsupported commerce claims',async()=>{
    await openMetadata();
    vi.useFakeTimers();
    const enhance=screen.queryByRole('button',{name:/Enhance with AI/i});
    if(enhance) fireEvent.click(enhance);
    await act(async()=>{vi.advanceTimersByTime(1600)});
    expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe('Twelve daily rainfall observations from a synthetic weather station.');
    expect(screen.getByText('rainfall')).toBeTruthy();
  });
  it('does not advertise AI generation or an estimated charge without a model action',async()=>{
    await openMetadata();
    expect(screen.queryByRole('button',{name:/Enhance with AI/i})).toBeNull();
    expect(screen.queryByText('~$0.03')).toBeNull();
    expect(screen.getByText('AI description enhancement is unavailable here. Describe the actual dataset.')).toBeVisible();
  });
  it('keeps manual metadata and navigation usable without publishing',async()=>{
    await openMetadata();
    fireEvent.click(screen.getByText('Next'));
    expect(screen.getByLabelText('Price (USD)')).toHaveValue(450);
    fireEvent.click(screen.getByText('Back'));
    expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toContain('rainfall observations');
    expect(screen.getByText('rainfall')).toBeTruthy();
    expect(screen.getByLabelText('Title')).toHaveValue('Synthetic rainfall observations');
    expect(h.scan).toHaveBeenCalledExactlyOnceWith('synthetic-weather');
  });
});
