## Technology Stack

- 客户端：`Tauri` + `TS` + `React` + `Rust`。

## Dependency Policy

- 默认减少第三方库依赖；如确实需要引入第三方库，先与用户确认。

## Code Organization

- 代码按功能模块拆分；避免单文件承载过多职责，避免超大文件。

## Code Style

- 开发过程中所有图标统一使用 `Heroicons` 的 `Solid` 版 `SVG` 图标。

## Git and Commit Rules

- 本地 git 提交信息使用中文。使用标准格式前戳表示调整类型。
- 新建 git 分支使用统一格式，如 feat/xxx、fix/xxx、refactor/xxx 的前戳加分支名格式。
- 进行 PR 合并时使用 squash merge，合并结束删除本地和远程分支。

## DO NOT

- 页面中禁止出现任何功能描述或解释性描述内容。
