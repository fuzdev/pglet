-- The set rule reads from the **index** side as well. `CREATE UNIQUE INDEX … (a, a)` is
-- accepted by both engines — a constraint's key list refuses a repeat, an index's does
-- not — and the index's own attribute set is then `{a}`, which `ON CONFLICT (a)` matches.
-- pglet compared the lists element-wise and answered `42P10`; the `(a, a)` spelling of
-- the same target matched, by the accident of the two lists being equal as sequences.
--
-- Enforcement over such an index agrees on both engines and is not what this asserts: a
-- second row with `a = 1` is `23505` on `oat_aa` either way.
-- @setup
CREATE TABLE oat (a INT, b INT);
CREATE UNIQUE INDEX oat_aa ON oat (a, a);
INSERT INTO oat VALUES (1, 1);
INSERT INTO oat VALUES (1, 5) ON CONFLICT (a) DO UPDATE SET b = 99;

-- @test
SELECT a, b FROM oat ORDER BY a;
