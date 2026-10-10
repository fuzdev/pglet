-- The textual-order control for `default_beats_a_duplicate_check_name.sql`: writing
-- the CHECK ahead of the defaulted column changes nothing, because the cook is a whole
-- pass ahead of the constraint *naming* block rather than interleaved with the column
-- list.
-- @setup
CREATE TABLE def_ck2_other (q INT CONSTRAINT def_dup_ck2 CHECK (q > 0));
-- @expect-error: textual order does not move the default ahead of or behind the block
-- @no-profile-lock: neither the @setup nor the @test builds an index — the refusal is raised while cooking the default, before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE def_ck2 (a INT CONSTRAINT def_dup_ck2 CHECK (a > 0), b INT DEFAULT 'x');
