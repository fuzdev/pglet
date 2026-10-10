-- Writing one is refused by both engines (`cannot change sequence`), and the SQLSTATE
-- is the same 42809 — so this half is conformance, not a divergence.
-- @setup
CREATE SEQUENCE seqins;
-- @test
INSERT INTO seqins VALUES (1);
-- @expect-error: a sequence is not a writable table (42809)
-- @no-error-fields: 18.6 answers this with `cannot change sequence "x"` and no DETAIL; pglet reaches it through the `cannot open relation` wording, which carries one. (The SQLSTATE agrees: `42501 permission denied` is the **non-superuser** answer — the sequence's own owner included, since a sequence carries no INSERT/DELETE ACL bit — and the harness connects as the superuser `postgres`, which bypasses the ACL and reaches the relkind check's `42809`, which is what this fixture's own recorded oracle output says.)
