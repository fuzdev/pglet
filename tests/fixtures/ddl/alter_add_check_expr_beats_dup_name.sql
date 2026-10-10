-- `ALTER TABLE … ADD CONSTRAINT … CHECK` runs the same cook-then-name order, so an
-- ill-typed predicate is 42883 even when its declared name is already taken. pglet
-- claimed the name first and answered 42710 — the neighbouring-gate trap: a clause with
-- no gate looks correct only because its neighbour's error wins by default, and adding
-- one changes the precedence.
-- @setup
CREATE TABLE aaced (a INT, CONSTRAINT aaced_dup CHECK (a > 0));

-- @expect-error: operator does not exist integer ~~ text
-- @test
ALTER TABLE aaced ADD CONSTRAINT aaced_dup CHECK (a LIKE 'x');
