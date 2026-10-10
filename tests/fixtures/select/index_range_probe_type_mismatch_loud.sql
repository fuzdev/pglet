-- The range half of the build-profile lock: an ordering comparison lifted into an
-- `IndexRangeScan` bypasses the operator gate the same way `=` does, so each bound
-- is gated where it is coerced. `integer > boolean` is `42883` on both profiles.
-- @profile-lock
-- @setup
CREATE TABLE ix_range (n INT);
CREATE INDEX ix_range_n ON ix_range (n);
INSERT INTO ix_range VALUES (0), (1), (2);
-- @test
-- @expect-error: no integer > boolean operator, on both build profiles
SELECT * FROM ix_range WHERE n > true;
