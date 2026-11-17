// The name of the TableScroll a table drawn from Markdown stands in (spec 02 section 3.1), so a listener hears what the
// table holds: its header cells' words ("Station, Fill pace"), read from the table's syntax tree (hast), else words the
// caller gives for a table without a header.

/** The parts of a Markdown element's syntax tree (hast) a table's name is read from. */
interface TreeNode {
  type: string;
  tagName?: string;
  value?: string;
  children?: TreeNode[];
}

const textOf = (node: TreeNode): string => (node.type === 'text' ? node.value ?? '' : (node.children ?? []).map(textOf).join(''));

export function tableName(table: TreeNode | undefined, otherwise: string): string {
  const head = table?.children?.find((child) => child.tagName === 'thead');
  const cells = head?.children?.find((row) => row.tagName === 'tr')?.children?.filter((cell) => cell.tagName === 'th') ?? [];
  const words = cells.map((cell) => textOf(cell).trim()).filter((word) => word !== '');
  return words.length > 0 ? words.join(', ') : otherwise;
}
