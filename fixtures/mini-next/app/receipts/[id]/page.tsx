import { getReceipts } from "@/lib/db";

export default async function ReceiptPage() {
  const all = await getReceipts();
  return <pre>{JSON.stringify(all)}</pre>;
}
