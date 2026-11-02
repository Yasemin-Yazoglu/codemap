export type Receipt = { id: string; date: string; amount: number };

export async function getReceipts(): Promise<Receipt[]> {
  return [];
}
