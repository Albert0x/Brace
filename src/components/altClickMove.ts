// Alt+点击把光标挪到点击处。
//
// 终端没法直接「设置光标位置」，只能替用户按方向键，让 shell 自己去挪。
// xterm 自带这个功能（altClickMovesCursor），但它按**格子**数方向键，
// 而 readline / PSReadLine 按一次 ← 只挪一个**字符**。中文、emoji 占两格，
// 光标前面每有一个宽字符就多走一格——命令行里有中文时几乎点不准。
// 所以这里自己数：只数真正占着字符的格子，宽字符的后半格不算。

export interface CellInfo {
  // 0 = 宽字符的后半格；1 = 普通字符；2 = 宽字符的前半格
  width: number;
  // 空串 = 从没写过内容的格子（行尾空白、宽字符换行时留下的填充格）
  chars: string;
}

export interface Grid {
  cols: number;
  // 这一行是不是上一行自动折过来的（同一条命令的延续）
  isWrapped(y: number): boolean;
  cell(x: number, y: number): CellInfo | undefined;
}

export interface Pos {
  x: number;
  y: number;
}

// 返回要发给 shell 的方向键序列；不该动时返回 null
export function altClickArrows(
  grid: Grid,
  cursor: Pos,
  target: Pos,
  applicationCursor: boolean,
): string | null {
  // 只在光标所在的这一条逻辑行里挪。点到别的行（上面的输出、上一条命令）
  // 就什么都不做：横着按方向键到不了那里，只会把光标挪到莫名其妙的位置
  let start = cursor.y;
  while (start > 0 && grid.isWrapped(start)) start--;
  let end = cursor.y;
  while (grid.isWrapped(end + 1)) end++;
  if (target.y < start || target.y > end) return null;

  // 点在宽字符的后半格上，算作点中这个字
  let tx = target.x;
  if (tx > 0 && grid.cell(tx, target.y)?.width === 0) tx--;

  const index = (x: number, y: number) => (y - start) * grid.cols + x;
  const from = index(cursor.x, cursor.y);
  const to = index(tx, target.y);
  if (from === to) return null;

  // 数两点之间有几个字符。空格子不算：行尾之后的空白本来就不是命令的一部分，
  // 不数它们，点到行尾右边就自然停在命令末尾，而不是按一串 → 让 shell 响铃
  const [lo, hi] = from < to ? [from, to] : [to, from];
  let chars = 0;
  for (let i = lo; i < hi; i++) {
    const c = grid.cell(i % grid.cols, start + Math.floor(i / grid.cols));
    if (c && c.width > 0 && c.chars !== "") chars++;
  }
  if (chars === 0) return null;

  const prefix = applicationCursor ? "\x1bO" : "\x1b[";
  return (prefix + (from < to ? "C" : "D")).repeat(chars);
}
