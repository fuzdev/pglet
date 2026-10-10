-- The `SET` **target** is a column of the relation, so a name it lacks is
-- `42703 column "nosuchcol" of relation "ocst" does not exist` — a different error from
-- the right-hand sides' bare `42703`, and it names the relation, as `attnameAttNum` does.
-- With no conflict this wrote its row.
--
-- Its accept polarity is any ordinary `SET v = excluded.v`, and its precedence neighbour
-- is `insert/on_conflict_action_rhs_precedes_target.sql`: a right-hand side is transformed
-- before *any* target is assigned, so a bad one beats this error.
-- @setup
CREATE TABLE ocst (n INT PRIMARY KEY, v TEXT);
INSERT INTO ocst VALUES (1, 'a');

-- @expect-error: column "nosuchcol" of relation "ocst" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocst VALUES (2, 'b') ON CONFLICT (n) DO UPDATE SET nosuchcol = 'z';
