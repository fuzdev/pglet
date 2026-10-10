-- The write this closes that no enum column guards: the projection's value is stored into
-- a **text** column, so the write path's own enum gate never sees it. Three rows landed
-- here where 18.6 lands none.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE tx (v TEXT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
INSERT INTO tx SELECT greatest(m, 'zzz') FROM t;
