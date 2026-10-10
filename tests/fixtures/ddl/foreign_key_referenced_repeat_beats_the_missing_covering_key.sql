-- And the other side: the repeat is tested before the covering-key lookup, so a parent
-- with no unique key on the repeated column reports the duplicate rather than *there is
-- no unique constraint matching given keys*. Both are `42830`, so `@expect-message` is
-- the whole assertion — the position decides the sentence and not the code.
-- @setup
CREATE TABLE fkck_p (x INT, y INT, UNIQUE (x));
-- @expect-error: the repeated referenced column outranks the missing covering key
-- @expect-message
-- @no-profile-lock: the @setup's index is the parent's enforced UNIQUE key; the refusal is a DDL-time catalog gate raised before any plan
-- @test
CREATE TABLE fkck_c (a INT, b INT, FOREIGN KEY (a, b) REFERENCES fkck_p (y, y));
