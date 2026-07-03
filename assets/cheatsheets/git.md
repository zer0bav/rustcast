# git — everyday commands

## Status & staging
git status
git add file        / git add -p       # stage / interactively
git restore file                        # discard unstaged changes
git restore --staged file               # unstage
git reset --hard                        # discard ALL local changes

## Commit & amend
git commit -m "msg"
git commit --amend                      # edit last commit
git commit --amend --no-edit            # add staged to last commit

## Branches
git switch -c feature                    # create & switch
git switch main
git branch -d feature                    # delete
git branch -a                            # list all

## Sync
git fetch --all
git pull --rebase
git push -u origin feature
git push --force-with-lease              # safer force push

## History & inspection
git log --oneline --graph --all
git diff            / git diff --staged
git show HEAD
git blame file

## Undo & rescue
git revert <commit>                      # new commit that undoes
git reset --soft HEAD~1                  # uncommit, keep changes staged
git reflog                               # find lost commits
git stash / git stash pop                # shelve / restore work
