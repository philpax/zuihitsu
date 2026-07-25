import { describe, expect, it } from "vitest";

import { collapseSameAs, type MemoryGraph, type MemoryGraphLink } from "./memoryGraph.ts";

function same(source: string, target: string): MemoryGraphLink {
  return {
    source,
    target,
    relation: "same as",
    same: true,
    visibility: "Public",
    told_by: null,
    told_in: null,
    asserted_at: null,
  };
}

function typed(source: string, target: string, relation: string): MemoryGraphLink {
  return {
    source,
    target,
    relation,
    same: false,
    visibility: "Public",
    told_by: null,
    told_in: null,
    asserted_at: null,
  };
}

function graphOf(nodeIds: string[], links: MemoryGraphLink[]): MemoryGraph {
  return {
    nodes: nodeIds.map((id) => ({ id, namespace: id.split("/")[0] ?? id })),
    links,
  };
}

describe("collapseSameAs", () => {
  // The live shape this guards: a digit-leading platform stub sorts lexicographically before its
  // canonical profile, so without the primary map the class node (and every edge routed through it)
  // is labelled by the stub instead of the identity the agent's own reads collapse to.
  const graph = graphOf(
    ["person/1234@platform", "person/dave", "topic/kites"],
    [
      same("person/dave", "person/1234@platform"),
      typed("topic/kites", "person/1234@platform", "created_by"),
    ],
  );

  it("labels a class by its primary, not the lexicographically smallest member", () => {
    const primaryOf = new Map([
      ["person/1234@platform", "person/dave"],
      ["person/dave", "person/dave"],
    ]);
    const collapsed = collapseSameAs(graph, primaryOf);
    const classNode = collapsed.nodes.find((node) => node.members);
    expect(classNode?.id).toBe("person/dave (2)");
    // The typed edge routes through the primary-labelled class node, staying attached to it.
    expect(collapsed.links).toEqual([
      expect.objectContaining({ source: "topic/kites", target: "person/dave (2)" }),
    ]);
  });

  it("falls back to the lexicographic head without a primary map", () => {
    const collapsed = collapseSameAs(graph);
    const classNode = collapsed.nodes.find((node) => node.members);
    expect(classNode?.id).toBe("person/1234@platform (2)");
  });

  it("ignores a primary that is not a member of the class", () => {
    const primaryOf = new Map([["person/1234@platform", "person/erin"]]);
    const collapsed = collapseSameAs(graph, primaryOf);
    const classNode = collapsed.nodes.find((node) => node.members);
    expect(classNode?.id).toBe("person/1234@platform (2)");
  });
});
