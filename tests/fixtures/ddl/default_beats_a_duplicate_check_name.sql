-- The other side of the cook's position: the constraint *naming* block runs **after**
-- it — not every constraint gate, since a constraint's key list is validated in the
-- pass above (`key_column_repeat_beats_the_column_default_cook.sql`) — so the default's
-- `22P02` beats a duplicate CHECK name's `42710`, even with the CHECK written first
-- (`…_check_written_first`). Sequenced anywhere ahead of the naming block this pair
-- reverses, which is what a same-code pair could not see.
-- @setup
CREATE TABLE def_ck_other (q INT CONSTRAINT def_dup_ck CHECK (q > 0));
-- @expect-error: the default is cooked before the constraints are named
-- @no-profile-lock: neither the @setup nor the @test builds an index — the refusal is raised while cooking the default, before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE def_ck (a INT DEFAULT 'x', b INT CONSTRAINT def_dup_ck CHECK (b > 0));
