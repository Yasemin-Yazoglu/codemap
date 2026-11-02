import { AnalysisChart } from "@/components/AnalysisChart";
import { buildMonthlyData } from "@/lib/analysis";
import { getReceipts } from "@/lib/db";

export default async function AnalysisPage() {
  const receipts = await getReceipts();
  return <AnalysisChart data={buildMonthlyData(receipts)} />;
}
