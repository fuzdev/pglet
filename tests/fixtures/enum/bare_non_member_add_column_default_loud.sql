-- The route that **stored** it (EDGES.md row 105): `ADD COLUMN` back-filled the existing row
-- with a label the enum does not hold, which then compared after every member.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE u (id INT);
INSERT INTO u VALUES (1);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE u ADD COLUMN m mood DEFAULT 'q';
