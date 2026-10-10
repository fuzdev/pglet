-- The positive twin of the two `on_conflict_set_*_loud` fixtures: gating the `SET`
-- right-hand side must not cost the unknown-literal coercion. The assignment is resolved
-- against the same scope the `DO UPDATE … WHERE` uses — the target relation plus the
-- `EXCLUDED` pseudo-relation — so `EXCLUDED.i4 = '10'` compares as integers rather than
-- being rejected as `integer = text`.
-- @setup
CREATE TABLE ocsl (id INT PRIMARY KEY, i4 INT4, b BOOL);
INSERT INTO ocsl VALUES (1, 10, false);
-- @test
INSERT INTO ocsl VALUES (1, 10, false) ON CONFLICT (id) DO UPDATE SET b = (EXCLUDED.i4 = '10') RETURNING id, b;
