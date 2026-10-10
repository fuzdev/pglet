-- The restart target is the sequence's `START WITH`, not zero. At the default start
-- the two answers coincide (`start - 1 == 0`), which is why a plain-SERIAL fixture
-- cannot see this.
-- @setup
CREATE TABLE tris_t (id INT GENERATED ALWAYS AS IDENTITY (START WITH 7), v TEXT);
INSERT INTO tris_t (v) VALUES ('a'), ('b');
TRUNCATE tris_t RESTART IDENTITY;
INSERT INTO tris_t (v) VALUES ('c');
-- @test
SELECT id, v FROM tris_t;
