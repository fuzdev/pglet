-- `setval` has two overloads — `(regclass, bigint)` and `(regclass, bigint, boolean)`
-- — resolved from the argument **types** during parse analysis, so a NULL that is
-- declared `text` is refused before strictness is ever consulted. pglet short-circuited
-- on the *value* (`Value::Null` carries no type) and returned NULL: a loud rejection
-- turned into a silent success.
-- @setup
CREATE SEQUENCE svt_seq;
-- @test
SELECT setval('svt_seq', NULL::text);
-- @expect-error: no setval takes a text second argument (42883)
