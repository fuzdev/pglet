-- Two empty-string keys still collide on a unique index (the empty string is an
-- ordinary value — `NULLS NOT DISTINCT` doesn't change that). This guards the
-- NULL-vs-'' distinction in `nulls_not_distinct_null_vs_empty_string` from over-
-- correcting: '' must stay a real, collidable value after the presence tag.
-- @profile-lock
-- @setup
CREATE TABLE nnd_two_empty (id INT, tag TEXT);
CREATE UNIQUE INDEX ux_nnd_two_empty ON nnd_two_empty (tag) NULLS NOT DISTINCT;
INSERT INTO nnd_two_empty VALUES (1, '');

-- @test
-- @expect-error: a second '' collides (the empty string is an ordinary unique value)
INSERT INTO nnd_two_empty VALUES (2, '');
