import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import FleetShell from '../FleetShell';

const { push } = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push }),
}));

const props = {
  hosts: [],
  snapshots: {},
  healthByAgent: {},
  liveStatus: 'live' as const,
  loading: false,
  nowSeconds: 1_000,
  refresh: vi.fn(),
};

describe('FleetShell', () => {
  afterEach(() => {
    cleanup();
    push.mockClear();
    props.refresh.mockClear();
  });

  it('runs snapshot commands and recalls them with ArrowUp', () => {
    render(<FleetShell {...props} />);
    const input = screen.getByLabelText('Fleet Shell command');
    fireEvent.change(input, { target: { value: 'stats' } });
    fireEvent.submit(screen.getByRole('form', { name: 'Run Fleet Shell command' }));

    expect(screen.getByText(/FLEET 0\/0 online/)).toBeInTheDocument();
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(input).toHaveValue('stats');
  });

  it('runs quick commands and routes only through typed effects', () => {
    render(<FleetShell {...props} />);
    fireEvent.click(screen.getByRole('button', { name: 'stats' }));
    expect(screen.getByText(/FLEET 0\/0 online/)).toBeInTheDocument();

    const input = screen.getByLabelText('Fleet Shell command');
    fireEvent.change(input, { target: { value: 'terminal' } });
    fireEvent.submit(screen.getByRole('form', { name: 'Run Fleet Shell command' }));
    expect(push).toHaveBeenCalledWith('/terminal');
  });

  it('requests a durable refresh without leaving the overview', () => {
    render(<FleetShell {...props} />);
    const input = screen.getByLabelText('Fleet Shell command');
    fireEvent.change(input, { target: { value: 'refresh' } });
    fireEvent.submit(screen.getByRole('form', { name: 'Run Fleet Shell command' }));
    expect(props.refresh).toHaveBeenCalledOnce();
    expect(push).not.toHaveBeenCalled();
  });
});
