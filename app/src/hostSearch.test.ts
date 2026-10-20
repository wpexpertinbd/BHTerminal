// Run with: node --test app/src/hostSearch.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { addressOf, folderPathOf, searchHosts } from "./hostSearch.ts";
import type { Host, HostFolder } from "./api";

let seq = 0;
function host(name: string, hostname: string, extra: Partial<Host> = {}): Host {
  return {
    id: `H${++seq}`,
    name,
    hostname,
    port: 22,
    username: "root",
    authMethod: { password: {} },
    connectionType: "ssh",
    folderID: null,
    jumpHostID: null,
    tunnels: [],
    sortOrder: 0,
    notes: "",
    ...extra,
  };
}

const folders: HostFolder[] = [
  { id: "F1", name: "BiswasHost", parentFolderID: null, sortOrder: 0 },
  { id: "F2", name: "Clients", parentFolderID: "F1", sortOrder: 0 },
  { id: "F3", name: "Unmanaged", parentFolderID: null, sortOrder: 1 },
];

const hosts = [
  host("s1.bitsboxhost.com", "65.108.43.154", { folderID: "F1", port: 2279 }),
  host("s3.bitsboxhost.com", "142.91.103.158", { folderID: "F1", port: 2279 }),
  host("shorifart.com", "23.111.14.99", { folderID: "F3" }),
  host("mail.shorifart.com", "23.111.14.98", { folderID: "F3" }),
  host("deep client", "10.0.0.5", { folderID: "F2", username: "deploy", notes: "staging box" }),
  host("pdns.bitsboxhost.com", "65.21.48.27", { folderID: "F1", port: 2279 }),
];
const names = (q: string) => searchHosts(q, hosts, folders).map((m) => m.host.name);

test("an empty query finds nothing (the tree is shown instead)", () => {
  assert.deepEqual(searchHosts("   ", hosts, folders), []);
});

test("finds by IP, full or partial, across every folder", () => {
  assert.deepEqual(names("23.111.14.99"), ["shorifart.com"]);
  assert.deepEqual(names("23.111"), ["mail.shorifart.com", "shorifart.com"]);
  assert.deepEqual(names("142.91"), ["s3.bitsboxhost.com"]);
});

test("finds by hostname and ranks the exact or leading match first", () => {
  assert.equal(names("shorifart")[0], "shorifart.com", "name starts with it beats a mid-name match");
  assert.deepEqual(names("shorifart").sort(), ["mail.shorifart.com", "shorifart.com"]);
  assert.equal(names("s3")[0], "s3.bitsboxhost.com");
});

test("every word must match; words can hit different fields", () => {
  assert.deepEqual(names("mail 23.111"), ["mail.shorifart.com"]);
  assert.deepEqual(names("bitsbox 2279").length, 3);
  assert.deepEqual(names("shorifart 65.108"), []);
});

test("finds hosts filed in a SUB-folder, and by folder name", () => {
  assert.deepEqual(names("deep"), ["deep client"]);
  assert.deepEqual(names("clients"), ["deep client"]);
  assert.equal(folderPathOf("F2", folders), "BiswasHost / Clients");
  assert.equal(searchHosts("deep", hosts, folders)[0].folderPath, "BiswasHost / Clients");
});

test("user, notes and an exact port count; a port fragment does not", () => {
  assert.deepEqual(names("deploy"), ["deep client"]);
  assert.deepEqual(names("deploy@10.0"), ["deep client"]);
  assert.deepEqual(names("staging"), ["deep client"]);
  assert.equal(names("2279").length, 3);
  assert.deepEqual(names("227"), [], "a fragment of a port must not match");
});

test("is case-insensitive", () => {
  assert.deepEqual(names("SHORIFART.COM"), ["shorifart.com", "mail.shorifart.com"]);
});

test("forgives missing letters in a longer word, but ranks it last", () => {
  assert.ok(names("shfrt").includes("shorifart.com"));
  assert.deepEqual(names("xz"), [], "short fragments are never fuzzy-matched");
  // Numbers match literally: 2…2…7 in order inside 65.21.48.27 is not a hit.
  assert.deepEqual(names("2127"), []);
  assert.deepEqual(names("65.27"), []);
});

test("reports what to highlight in the name and the address line", () => {
  const m = searchHosts("23.111.14", hosts, folders).find((x) => x.host.name === "shorifart.com")!;
  const addr = addressOf(m.host);
  assert.equal(addr, "root@23.111.14.99:22");
  assert.deepEqual(m.addressHits, [[5, 14]]);
  assert.equal(addr.slice(5, 14), "23.111.14");
  const n = searchHosts("fart", hosts, folders).find((x) => x.host.name === "mail.shorifart.com")!;
  assert.deepEqual(n.nameHits, [[10, 14]]);
  assert.equal(n.host.name.slice(10, 14), "fart");
});

test("a name whose lower-case form changes length is found but not mis-highlighted", () => {
  const odd = [host("İx server", "10.9.9.9")];
  const [m] = searchHosts("server", odd, []);
  assert.ok(m, "still found");
  assert.deepEqual(m.nameHits, [], "no highlight rather than a shifted one");
  assert.deepEqual(m.addressHits, [], "the address line has no match for this word");
});

test("a folder cycle cannot hang the path builder", () => {
  const loop: HostFolder[] = [
    { id: "A", name: "a", parentFolderID: "B", sortOrder: 0 },
    { id: "B", name: "b", parentFolderID: "A", sortOrder: 0 },
  ];
  assert.equal(folderPathOf("A", loop), "b / a");
});
