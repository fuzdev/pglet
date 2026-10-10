-- A `VALUES` list typed `real` by a sibling converts the literal and refuses it.
-- @setup
CREATE TABLE fio (r REAL);
-- @test
-- @expect-error: "1e39" is out of range for type real (22003)
INSERT INTO fio (r) SELECT x FROM (VALUES (1::real), ('1e39')) v(x);
