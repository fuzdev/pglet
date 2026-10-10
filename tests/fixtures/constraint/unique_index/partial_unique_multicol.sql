-- A COMPOSITE partial unique index (`(a, b) WHERE revoked_at IS NULL`): an active
-- full-tuple duplicate is rejected, while a duplicate where a row is revoked
-- (fails the predicate) is allowed. This test inserts the active dup → error.
-- @profile-lock
-- @setup
CREATE TABLE partial_multicol (a INT, b TEXT, revoked_at TIMESTAMPTZ);
CREATE UNIQUE INDEX ux_partial_multicol ON partial_multicol (a, b) WHERE revoked_at IS NULL;
-- (1,'x') active; (1,'x') revoked is allowed (fails predicate); (2,'x') active.
INSERT INTO partial_multicol VALUES (1, 'x', NULL), (1, 'x', '2020-01-01T00:00:00Z'), (2, 'x', NULL);

-- @test
-- @expect-error: duplicate active composite tuple (1, 'x') violates the partial unique index
INSERT INTO partial_multicol VALUES (1, 'x', NULL);
