-- An `INSERT` into a `timestamptz` column refuses an out-of-range offset rather than
-- storing a shifted instant.
-- @setup
CREATE TABLE ev (id INT, at TIMESTAMPTZ);
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
INSERT INTO ev VALUES (1, '2000-01-01 00:00:00-16');
