import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// globals: false 时 testing-library 不会自己挂 cleanup，手动挂上。
// localStorage 也要清——usePersisted / useTabs 都往里写，不清会串味
afterEach(() => {
  cleanup();
  localStorage.clear();
});
