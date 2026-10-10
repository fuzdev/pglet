-- `ALTER TABLE … ADD COLUMN … TEXT[] … UNIQUE` with a default over an existing row
-- back-fills the key, and the index then refuses its duplicate.
-- @profile-lock
-- @setup
CREATE TABLE ckl_t (id INT);
INSERT INTO ckl_t VALUES (1);
ALTER TABLE ckl_t ADD COLUMN tags TEXT[] DEFAULT '{x,y}' UNIQUE;
-- @test
-- @expect-error: a duplicate of the back-filled default (23505)
-- @expect-message
INSERT INTO ckl_t (tags, id) VALUES ('{x,y}', 2);
