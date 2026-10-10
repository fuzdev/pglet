-- The **accept** polarity of `returning_from_unnest_source_column_ambiguous.sql`.
--
-- `id` is a name the `unnest` source does not carry, so it binds — on 18.6 and here.
-- The alternative shape considered for this gap was to *seal* the scope whenever the
-- `FROM` held a relation the walk could not enumerate, refusing every bare name; that
-- shape reddens this file while turning its sibling green, and it would have introduced
-- a fresh breach of the rule `docs/ddl.md` § A gate standing in for *no* Postgres error
-- must run after every gate that stands in for one — a refusal standing in for no
-- Postgres error, raised at plan time, ahead of gates that stand in for real ones.
-- @setup
CREATE TABLE ruu_t (id INT, v TEXT);
INSERT INTO ruu_t VALUES (1, 'TARGET');

-- @test
UPDATE ruu_t SET id = 9 FROM unnest(ARRAY['x', 'y']) AS v RETURNING id;
