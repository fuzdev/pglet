-- The second accept-side pin: `cell_visibility`'s declared order **is** its lexical
-- order, so member-position ordering and label-byte ordering agree. The spine's own
-- enum is this one, which is why the divergence never showed on its traffic — and why
-- an over-correction that reversed or permuted the member order reddens here.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL);
INSERT INTO cell VALUES (1, 'public'), (2, 'private'), (3, 'public');

-- @test
SELECT id, visibility::text AS v FROM cell ORDER BY visibility, id;
