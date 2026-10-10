-- The refusal is where the value is written: nothing is stored.
-- @setup
CREATE TABLE fio (r REAL);
-- @test
-- @expect-error: "1e39" is out of range for type real (22003)
-- @expect-message
INSERT INTO fio (r) VALUES ('1e39');
