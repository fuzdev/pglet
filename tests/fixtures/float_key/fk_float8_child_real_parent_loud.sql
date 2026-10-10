-- A `float8` child of a `real` parent is compared by 18.6's cross-width `=`, both
-- sides as `float8`: the `float8` `1.1` is not the `real` `1.1`. Narrowing the child to
-- the parent's type to probe its index would round it onto that parent and accept.
-- @profile-lock
-- @expect-error: the float8 1.1 has no real parent (23503)
-- @expect-message
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkc84 (x FLOAT8 REFERENCES fkp4 (x));
-- @test
INSERT INTO fkc84 VALUES (1.1);
