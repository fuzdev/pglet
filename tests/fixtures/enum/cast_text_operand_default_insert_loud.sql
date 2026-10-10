-- …and refuses the first insert that takes it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE md (id INT, m mood DEFAULT 'q'::text::mood);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
INSERT INTO md (id) VALUES (1);
