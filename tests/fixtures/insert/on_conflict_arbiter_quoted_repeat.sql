-- The set is over the **columns**, so the quoted spelling of a repeat collapses with the
-- bare one. Both sides went through the parser's fold, so there is no casing left to
-- reconcile — a quoted name that folds to something else is a *different* column and
-- still refuses (`identifier/on_conflict_set_target_is_exact_loud.sql`).
-- @setup
CREATE TABLE oarq (a INT PRIMARY KEY, b INT);
INSERT INTO oarq VALUES (1, 1);
INSERT INTO oarq VALUES (1, 9) ON CONFLICT ("a", a) DO NOTHING;

-- @profile-lock
-- @test
SELECT a, b FROM oarq ORDER BY a;
