-- **A control — do not trim it.** The same inner scope as
-- `select/correlated_outer_ref_dotted_columns_not_ambiguous.sql` with no outer level to
-- resolve into: `"a.c"` and `"b.c"` are two ordinary columns, neither named `c`, so 18.6
-- says the column does not exist rather than that it is ambiguous.
--
-- What answers it here is the **plan-time** binding gate, which counts claimants over the
-- FROM namespace and finds none — so this stays green under every mutation of the
-- executor's pair-keyed resolution, including the suffix-matching near-miss its sibling
-- exists to catch. It is a control on the pair, not a guard on the executor.
-- @setup
CREATE TABLE dcbm_i ("a.c" INT, "b.c" INT);
CREATE TABLE dcbm_z (z INT);
INSERT INTO dcbm_i VALUES (10, 20);
INSERT INTO dcbm_z VALUES (99);

-- @expect-error: column "c" does not exist (42703)
-- @test
SELECT 1 FROM dcbm_i, dcbm_z WHERE c = 1;
