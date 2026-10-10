-- The most visibly wrong case: a `bool` declared `text` renders `t` either way, so
-- the value comparison passes while every driver that types by the declaration
-- hands its caller the string `"t"` instead of `true`. Alias-qualified (`q.flag`
-- against `FROM qbd q`) so the alias-resolution tier is covered too.
-- @profile-lock
-- @setup
CREATE TABLE qbd (id INT PRIMARY KEY, flag BOOL);
INSERT INTO qbd VALUES (1, true), (2, false), (3, NULL);

-- @test
SELECT q.id, q.flag FROM qbd q ORDER BY q.id;
