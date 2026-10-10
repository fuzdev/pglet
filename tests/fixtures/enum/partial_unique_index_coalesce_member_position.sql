-- `partial_unique_index_member_position` one wrapper deeper: the predicate's enum
-- identity comes through a `COALESCE`, so before the walk became exhaustive the index
-- decided membership by label bytes — neither row was placed in it, and the duplicate
-- landed with no error. This is the silent-write shape, and the one `docs/enums.md`
-- claimed the member-order rule had already closed.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE pt (id INT, m mood);
CREATE UNIQUE INDEX pt_u ON pt (id) WHERE COALESCE(m, 'sad') > 'ok';
INSERT INTO pt VALUES (1, 'happy');

-- @profile-lock
-- @test
-- @expect-error: both rows satisfy the partial predicate, so the duplicate id collides
INSERT INTO pt VALUES (1, 'happy');
