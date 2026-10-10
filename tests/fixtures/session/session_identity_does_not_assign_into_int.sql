-- The mirror rule: a string-category **source** needs an explicit cast, so `name`
-- into an `integer` column is `42804` — the polarity that keeps the `is_string`
-- membership from being a wildcard.
-- @expect-error: name does not assign into integer
-- @setup
CREATE TABLE sa_reject (i INT);
-- @test
INSERT INTO sa_reject (i) VALUES (current_user);
