-- `ATSimplePermissions` refuses the ALTER on a sequence before the constraint is
-- cooked, so the relkind beats the CHECK's unresolvable column (42809, not 42703).
-- pglet's `ADD CHECK` kept its own relation lookup two statements above the shared
-- one, so the shared one was dead here and the answer was 42P01.
-- @setup
CREATE SEQUENCE atc_seq;
-- @test
ALTER TABLE atc_seq ADD CHECK (nosuchcol > 0);
-- @expect-error: ALTER TABLE cannot be performed on a sequence (42809)
