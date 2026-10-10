-- An `UPDATE … FROM` naming a relation the catalog lacks is `42P01` on 18.6, from the
-- range table ahead of parse analysis, and pglet raises it at execution — downstream of
-- every plan-time verdict. So this gate takes the same skip the assignment cast and the
-- identity guard take, and the executor's `42P01` stands.
-- @setup
CREATE TABLE dsms (a INT, b INT);
INSERT INTO dsms VALUES (0, 0);

-- @expect-error: relation "nosuchrel" does not exist (42P01)
-- @no-profile-lock: the refusal is raised at execution, and the statement plans no index scan
-- @test
UPDATE dsms SET a = 1, a = 2 FROM nosuchrel;
