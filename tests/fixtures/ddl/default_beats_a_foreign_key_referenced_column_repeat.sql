-- The other side of the same question, and the control against generalizing "a key
-- list outranks the cook": a foreign key's *referenced* list is validated in the
-- constraint block **behind** the cook, so the default's `22P02` wins over its `42830`
-- — where a PK/UNIQUE key list, validated in the pass above, wins over the default
-- (`key_column_repeat_beats_the_column_default_cook.sql`).
-- @setup
CREATE TABLE dfkrr_p (x INT, y INT, PRIMARY KEY (x, y));
-- @expect-error: the default is cooked before the foreign key's referenced list
-- @no-profile-lock: the @setup's index is the parent's enforced PRIMARY KEY, built so the referenced list has something to resolve against; the refusal is raised while cooking the default, before any plan
-- @test
CREATE TABLE dfkrr_c (a INT DEFAULT 'zz', b INT, FOREIGN KEY (a, b) REFERENCES dfkrr_p (x, x));
