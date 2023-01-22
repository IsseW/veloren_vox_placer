
# Veloren Vox Placer.

This is a simple program to load voxel assets directly into the persistence format veloren uses. 

# Usage

The program is configured through the `ron` file `assets/place.ron`.

The ron file has 3 fields.


### `pieces`
Pieces defines vox file to load and the position they should be placed at. It is an array of tuples, the first element being the path to the asset (uses the same asset path system as veloren). And the second element being the offset, a tuple of three signed integers. The offset describes the position of the corner with the lowest coordinates.

### `replace`
Replace is optional and defines colors that should be relaced with special blocks. It is an array of tuples. The first element if the color to replace, which is a tuple with 3 elements, representing rgb. The second element defines what block it should be replaced with, there are 3 different kinds of ways to define this with `BlockSpec`.

- `Sprite(kind: <insert sprite kind>, <optional> medium: <Air or Water>)`
You can find sprite kinds [here](https://docs.veloren.net/veloren_common/terrain/sprite/enum.SpriteKind.html).
- `Block(kind: <insert block kind here>, <optional> color: (0, 0, 0))` You can find different block kinds [here](https://docs.veloren.net/veloren_common/terrain/block/enum.BlockKind.html).
- `Random([(<weight>, <BlockSpec>), ...])` this works the same way as [`Lottery`](https://docs.veloren.net/veloren_common/lottery/struct.Lottery.html). It will randomly choose a block in the array, and the chance of a certain block is it's weight divided by the total weight of every entry in the array.

### `fill_empty`
can be `true` or `false`, defaults to `false`. If true empty voxels in the model will be written as air to persistance.


I advice that you run the program with release mode (`cargo run --release`). Since this program can be quite heavy, especially for large models.