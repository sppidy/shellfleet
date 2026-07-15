'use client';

import { Loader2Icon } from 'lucide-react';
import { useEeFeatures } from '@/lib/useEeFeatures';
import { hasFeature } from '@/lib/eeFeatures';
import EeUpsell from './EeUpsell';

/**
 * Renders `children` only when the EE license advertises `feature`; otherwise
 * shows the upsell (mirrors the server-side 402 gate). Drop this inside a
 * page's content/scroll area.
 */
export default function EeFeatureGate({
  feature,
  label,
  children,
}: {
  feature: string;
  label: string;
  children: React.ReactNode;
}) {
  const { features, loading } = useEeFeatures();
  if (loading) {
    return (
      <div className="empty">
        <Loader2Icon className="w-5 h-5 animate-spin" />
      </div>
    );
  }
  if (!hasFeature(features, feature)) {
    return <EeUpsell feature={feature} label={label} />;
  }
  return <>{children}</>;
}
