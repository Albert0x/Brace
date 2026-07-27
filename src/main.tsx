import ReactDOM from "react-dom/client";
import App from "./App";

// 注意：不用 React.StrictMode。
// StrictMode 在 dev 下会双调用 effect（mount→unmount→mount），
// 而我们的终端 effect 会创建/关闭后端 pty 这类副作用资源，
// 双调用会导致 pty 重复创建与关闭的竞态，故此处关闭。
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<App />);
