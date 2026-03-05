# DVS: Benchmarks

Claude:

>   Workflow: install old dvs → run.sh rpkg → install new dvs → run.sh rpkg (or cli) → compare the two CSVs.

Devin:

> 
> using devcluster - create a project/git repo with script(s) in it that generate data that is approximately 1, 5, 10, 50, 100, 500, 1000, 10000MB in size. This does not need to be exact, just in the ballpark. 
> initialize dvs from existing r package that points to /data/dvs/<project>. 
> use tictoc/etc to record the time taken to add each individual file
> use same to record status (essentially measure of hashing performance) - do this one in at least 5x replicate - be sure to not do the same file 5x in a row as that can result in caching, instead loop through each file to collect results, then do it again from the top - still could cache but more approximate to real life touching other files)
>
> replicate the same behavior, for 1, 5, 10, 50, 100 data sets (not 1000/10000) but make 20 generated datasets of each - now testing the parallelized performance.
>
> then perform the same using the new CLI.
>
> collect the raw results + summary metrics of the outcomes.
>
> My hypothesis is new dvs will be slower than old dvs, because we're using some extra tricks for hashing around memory mapping files etc, but i'm not sure what the differential will be - based on the difference we can determine whether to pull those capabilities in
>

