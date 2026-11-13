// The address of the Gantry the browser tests and the manual screenshots talk to:
//   1. GANTRY_E2E_URL, when set;
//   2. on your computer, with the Vagrant machine up: the port Vagrant forwarded to its
//      Gantry (written to .vagrant/gantry-ports.json after each `vagrant up`);
//   3. otherwise Gantry on this machine, on the port GANTRY_BIND names (8600 by default).
const fs = require("fs");
const path = require("path");

function baseUrl() {
  if (process.env.GANTRY_E2E_URL) return process.env.GANTRY_E2E_URL.replace(/\/$/, "");
  try {
    const vm = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "..", ".vagrant", "gantry-ports.json"), "utf8"));
    if (vm.app) return `http://localhost:${vm.app}`;
  } catch {
    // no Vagrant machine here (or this is the machine itself, which has no .vagrant folder)
  }
  const port = (process.env.GANTRY_BIND || "127.0.0.1:8600").split(":").pop();
  return `http://127.0.0.1:${port}`;
}

module.exports = { baseUrl };
