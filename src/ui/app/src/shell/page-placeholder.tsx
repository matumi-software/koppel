import { PageHeader } from '@heartwood/ui';

/**
 * Stands in for a page that is not built on Heartwood yet, so the app compiles and navigates. A page
 * replaces its placeholder by exporting the same component name from the same file.
 */
export function PagePlaceholder({ title }: { title: string }) {
  return (
    <div className="kp-page">
      <PageHeader title={title} description="This page is not built yet." />
    </div>
  );
}
