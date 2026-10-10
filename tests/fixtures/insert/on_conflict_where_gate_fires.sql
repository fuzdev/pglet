-- The `ON CONFLICT … DO UPDATE … WHERE` predicate is now gated at plan time, not
-- merely coercion-checked: Postgres rejects `text = integer` there when it parses the
-- statement, so the verdict must not depend on whether a conflicting row exists to
-- evaluate the predicate against.
--
-- The inserted `id` deliberately does **not** conflict, so `DO UPDATE` never runs and
-- the eval-time gate is never reached. Without the static gate this statement inserted
-- a row and reported success where Postgres raises `42883`.
-- @setup
CREATE TABLE ocg (id INT PRIMARY KEY, label TEXT);
INSERT INTO ocg VALUES (1, 'a');
-- @profile-lock
-- @expect-error: text = integer has no operator in the DO UPDATE WHERE (42883)
-- @test
INSERT INTO ocg VALUES (2, 'b') ON CONFLICT (id) DO UPDATE SET label = 'c' WHERE ocg.label = 5;
