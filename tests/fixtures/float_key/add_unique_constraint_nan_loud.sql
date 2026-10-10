-- `ADD CONSTRAINT … UNIQUE` over rows holding two NaNs of different sign.
-- @profile-lock
-- @expect-error: the build finds NaN beside -NaN (23505)
-- @expect-message
-- @setup
CREATE TABLE fkd (x FLOAT8);
INSERT INTO fkd VALUES ('NaN'), ('-NaN'::float8);
-- @test
ALTER TABLE fkd ADD CONSTRAINT fkd_u UNIQUE (x);
