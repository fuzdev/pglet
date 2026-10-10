-- Over an empty table too: the default is converted as it is cooked, not per row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE u (id INT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE u ADD COLUMN m mood DEFAULT 'q';
