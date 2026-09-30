import { describe, expect, it } from "vitest";
import { altClickArrows, type CellInfo, type Grid } from "./altClickMove";

// 按 xterm 的规矩把一行文字排进 cols 宽的网格：中文占两格；
// 宽字符在行尾放不下时，留一个空的填充格，整个字挪到下一行
// 测试只用到常见中日韩字符和全角符号，粗略按区段判断就够
const isWide = (ch: string) => /[\u3000-\u9fff\uff00-\uffef]/.test(ch);

function layout(text: string, cols: number, rowsBefore: string[] = []) {
  const rows: CellInfo[][] = rowsBefore.map((r) =>
    Array.from({ length: cols }, (_, i) => ({ width: 1, chars: r[i] ?? "" })),
  );
  const first = rows.length;
  const wrapped = new Set<number>();
  // 每个字符的起始格，posOf(i) 用
  const starts: { x: number; y: number }[] = [];
  let row: CellInfo[] = [];
  const newRow = () => {
    while (row.length < cols) row.push({ width: 1, chars: "" });
    rows.push(row);
    row = [];
    wrapped.add(rows.length);
  };
  for (const ch of text) {
    const w = isWide(ch) ? 2 : 1;
    if (row.length + w > cols) newRow();
    starts.push({ x: row.length, y: rows.length });
    row.push({ width: w, chars: ch });
    if (w === 2) row.push({ width: 0, chars: "" });
  }
  starts.push({ x: row.length, y: rows.length }); // 行尾，光标通常在这
  while (row.length < cols) row.push({ width: 1, chars: "" });
  rows.push(row);
  wrapped.delete(first);

  const grid: Grid = {
    cols,
    isWrapped: (y) => wrapped.has(y),
    cell: (x, y) => rows[y]?.[x],
  };
  return { grid, posOf: (i: number) => starts[i] };
}

const left = (n: number) => "\x1b[D".repeat(n);
const right = (n: number) => "\x1b[C".repeat(n);

describe("Alt+点击移动光标", () => {
  it("纯 ASCII：按字符数发方向键", () => {
    const { grid, posOf } = layout("$ git status", 80);
    // 光标在行尾，点到 git 的 g（第 2 个字符）
    expect(altClickArrows(grid, posOf(12), posOf(2), false)).toBe(left(10));
  });

  it("前面有中文：一个字只算一步，而不是两格两步", () => {
    // xterm 自带的实现在这里会发 left(8)——多出来的 4 步正是那 4 个中文字
    const { grid, posOf } = layout('$ git commit -m "修复粘贴"', 80);
    // 光标在行尾，点到 -m 的 -（第 13 个字符）。中间是 `-m "修复粘贴"` 共 9 个字符
    expect(altClickArrows(grid, posOf(22), posOf(13), false)).toBe(left(9));
  });

  it("点在中文字的后半格，算点中这个字", () => {
    const { grid, posOf } = layout("$ 中文ab", 80);
    const zhong = posOf(2);
    const back = { x: zhong.x + 1, y: zhong.y };
    expect(altClickArrows(grid, posOf(6), back, false)).toBe(left(4));
  });

  it("向右挪同样按字符数", () => {
    const { grid, posOf } = layout("$ 中文ab", 80);
    expect(altClickArrows(grid, posOf(2), posOf(5), false)).toBe(right(3));
  });

  it("点到行尾右边的空白：停在命令末尾，不多按", () => {
    const { grid, posOf } = layout("$ ab", 80);
    expect(altClickArrows(grid, posOf(2), { x: 50, y: 0 }, false)).toBe(right(2));
  });

  it("长命令折行后跨行点击", () => {
    // 10 列宽：`$ abcdefgh` 占满第一行，`ijkl` 在第二行
    const { grid, posOf } = layout("$ abcdefghijkl", 10);
    expect(grid.isWrapped(1)).toBe(true);
    // 光标在第二行末尾，点回第一行的 c
    expect(altClickArrows(grid, posOf(14), posOf(4), false)).toBe(left(10));
  });

  it("中文在行尾放不下时留下的填充格不算字符", () => {
    // 9 列：`$ abcdef` 占 8 格，`中` 放不下，第 9 格是填充格，`中` 挪到下一行
    const { grid, posOf } = layout("$ abcdef中x", 9);
    expect(posOf(8)).toEqual({ x: 0, y: 1 });
    // 光标在行尾（x 后面），点到 f：中间是 f、中、x 三个字符
    expect(altClickArrows(grid, posOf(10), posOf(7), false)).toBe(left(3));
  });

  it("点到上面的输出行：不动", () => {
    const { grid, posOf } = layout("$ ls", 80, ["some output"]);
    expect(altClickArrows(grid, posOf(4), { x: 3, y: 0 }, false)).toBeNull();
  });

  it("点在光标上：不动", () => {
    const { grid, posOf } = layout("$ ls", 80);
    expect(altClickArrows(grid, posOf(4), posOf(4), false)).toBeNull();
  });

  it("应用光标键模式用 ESC O", () => {
    const { grid, posOf } = layout("$ ab", 80);
    expect(altClickArrows(grid, posOf(4), posOf(2), true)).toBe("\x1bOD\x1bOD");
  });
});
