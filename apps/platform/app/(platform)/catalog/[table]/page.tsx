import { CatalogTableClient } from "./catalog-table-client";

export function generateStaticParams() {
  return [{ table: "passages" }];
}

export default async function CatalogTablePage({
  params,
}: {
  params: Promise<{ table: string }>;
}) {
  const { table } = await params;
  return <CatalogTableClient tableName={table} />;
}
