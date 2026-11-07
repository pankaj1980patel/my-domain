// The page's entry in screenshot mode (vite --mode mock; see vite.config.ts):
// with ?mock=1 it installs the sample backend first (src/dev/mockTauri.ts),
// then starts the app as usual. The normal build starts at src/index.tsx
// and never includes this.

const params = new URLSearchParams(location.search);

const start = () => import("../index");

if (params.get("mock") === "1") {
  import("./mockTauri").then((m) => {
    m.installMock(params);
    return start();
  });
} else {
  start();
}
