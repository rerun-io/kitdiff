# kitdiff, a visual diffing tool

I got frustrated with the experience of reviewing image snapshot changes in my ide and on github, so I made something really cool:

Just run `kitdiff` in your terminal and it'll visualize all the diffs! You can use the `1, 2, 3` keys to switch between `original, new, diff` images. You can also generate diffs on the fly to play with different threshold options. Also, it's wicked quick!

https://github.com/user-attachments/assets/c9324ef3-eb24-481f-83b8-42a37b6b075d

## but wait, there's more

You can do `kitdiff pr https://github.com/rerun-io/rerun/pull/11253` to view a diff of that PR, you don't even need to check out the branch!


https://github.com/user-attachments/assets/d5c0b15a-0a75-4506-8dae-51b8bb83836f


## Getting started

Just do a `cargo install --git https://github.com/rerun-io/kitdiff ` to install the binary

