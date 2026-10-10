-- The `ALTER TABLE … ADD CONSTRAINT` route reaches the same resolver, so it takes the
-- same refusal. Unlike the PK/UNIQUE repeat, this one has no route asymmetry: 18.6
-- raises it from the constraint's execution on both spellings.
-- @setup
CREATE TABLE acfk_p (x INT, y INT, UNIQUE (x, y), UNIQUE (x));
CREATE TABLE acfk_c (a INT, b INT);
-- @expect-error: the referenced list names x twice
-- @expect-message
-- @no-profile-lock: the @setup's indexes are the parent's enforced UNIQUE keys; the refusal is a DDL-time catalog gate raised before any plan
-- @test
ALTER TABLE acfk_c ADD CONSTRAINT acfk_fk FOREIGN KEY (a, b) REFERENCES acfk_p (x, x);
