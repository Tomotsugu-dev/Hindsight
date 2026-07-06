//! rec.onnx 的运行时 ArgMax 内存改图。
//!
//! PP-OCR 官方 rec 导出输出 `[N,T,C]` f32 概率——通用发布要保留概率给各种
//! 后处理(置信过滤/beam search),而本应用的 CTC 解码从第一天起就只取
//! argmax。把 ArgMax(axis=2) 追加进图后:argmax 由 GPU/优化 kernel 完成,
//! 输出缩成 `[N,T]` int64(GPU→CPU 拷贝从 ~190MB/批 降到 KB 级),实测
//! 整帧 -150ms(独显)/-120ms(CPU)。
//!
//! **只在内存里改,不写回磁盘**:磁盘永远是官方原件——天然幂等、新装与
//! 存量用户统一覆盖、官方升级模型自动继承、不存在写坏文件的风险面。
//! patch 失败返回原字节,加载方走 f32 慢路径(行为 = 引入本优化之前)。
//!
//! 与 `scripts/ocr/rec_argmax.py`(离线验证脚本,同 poc 脚本惯例 gitignored、
//! 仅存本地)逻辑一一对应;改这里需同步跑一次那边的等价性验证。

use onnx_protobuf::{AttributeProto, ModelProto, NodeProto, TypeProto, ValueInfoProto};
use protobuf::Message;

use onnx_protobuf::attribute_proto::AttributeType;
use onnx_protobuf::tensor_proto::DataType;
use onnx_protobuf::type_proto::Value as TypeValue;

/// rec 模型输出若仍是 f32 概率,就在内存里追加 ArgMax 节点;已是 int64
/// (上游哪天自带了)或解析失败则原样返回。返回 (bytes, 是否已为 argmax 形态)。
pub fn ensure_rec_argmax(bytes: Vec<u8>) -> (Vec<u8>, bool) {
    let mut model = match ModelProto::parse_from_bytes(&bytes) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("rec.onnx 解析失败,走 f32 慢路径: {e}");
            return (bytes, false);
        }
    };
    let graph = match model.graph.as_mut() {
        Some(g) => g,
        None => {
            log::warn!("rec.onnx 无 graph,走 f32 慢路径");
            return (bytes, false);
        }
    };
    let Some(out) = graph.output.first() else {
        log::warn!("rec.onnx 无输出声明,走 f32 慢路径");
        return (bytes, false);
    };

    // 已是 int64 输出(argmax 形态)→ 无需改
    let elem_type = out
        .type_
        .as_ref()
        .and_then(|t| match &t.value {
            Some(TypeValue::TensorType(tt)) => Some(tt.elem_type),
            _ => None,
        })
        .unwrap_or(0);
    if elem_type == DataType::INT64 as i32 {
        return (bytes, true);
    }
    if elem_type != DataType::FLOAT as i32 {
        log::warn!("rec.onnx 输出类型未知({elem_type}),走 f32 慢路径");
        return (bytes, false);
    }

    let src_name = out.name.clone();

    // ArgMax(axis=2, keepdims=0):[N,T,C] f32 → [N,T] int64
    let mut axis = AttributeProto::new();
    axis.name = "axis".into();
    axis.type_ = AttributeType::INT.into();
    axis.i = 2;
    let mut keepdims = AttributeProto::new();
    keepdims.name = "keepdims".into();
    keepdims.type_ = AttributeType::INT.into();
    keepdims.i = 0;
    let mut node = NodeProto::new();
    node.name = "ctc_argmax".into();
    node.op_type = "ArgMax".into();
    node.input.push(src_name);
    node.output.push("ctc_index".into());
    node.attribute.push(axis);
    node.attribute.push(keepdims);
    graph.node.push(node);

    // 输出声明替换为 int64 [N,T](维度留动态,ort 按实际形状跑)
    let mut vi = ValueInfoProto::new();
    vi.name = "ctc_index".into();
    let mut tp = TypeProto::new();
    let mut tt = onnx_protobuf::type_proto::Tensor::new();
    tt.elem_type = DataType::INT64 as i32;
    tp.value = Some(TypeValue::TensorType(tt));
    vi.type_ = protobuf::MessageField::some(tp);
    graph.output.clear();
    graph.output.push(vi);

    match model.write_to_bytes() {
        Ok(patched) => {
            log::info!("rec.onnx 已内存改图:输出 [N,T,C] f32 → ArgMax → [N,T] int64");
            (patched, true)
        }
        Err(e) => {
            log::warn!("rec.onnx 改图序列化失败,走 f32 慢路径: {e}");
            (bytes, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手造最小 rec 形状的 ModelProto:验证 patch 追加节点/替换输出,以及幂等。
    #[test]
    fn patch_appends_argmax_and_is_idempotent() {
        let mut model = ModelProto::new();
        let mut graph = onnx_protobuf::GraphProto::new();
        let mut out = ValueInfoProto::new();
        out.name = "softmax_0".into();
        let mut tp = TypeProto::new();
        let mut tt = onnx_protobuf::type_proto::Tensor::new();
        tt.elem_type = DataType::FLOAT as i32;
        tp.value = Some(TypeValue::TensorType(tt));
        out.type_ = protobuf::MessageField::some(tp);
        graph.output.push(out);
        model.graph = protobuf::MessageField::some(graph);
        let bytes = model.write_to_bytes().unwrap();

        let (patched, ok) = ensure_rec_argmax(bytes);
        assert!(ok);
        let m = ModelProto::parse_from_bytes(&patched).unwrap();
        let g = m.graph.as_ref().unwrap();
        assert_eq!(g.node.last().unwrap().op_type, "ArgMax");
        assert_eq!(g.output[0].name, "ctc_index");

        // 幂等:再 patch 一次应识别为已是 argmax 形态,原样返回
        let (again, ok2) = ensure_rec_argmax(patched.clone());
        assert!(ok2);
        assert_eq!(again, patched);
    }

    /// 垃圾字节:解析失败原样返回、标记走慢路径,绝不 panic。
    #[test]
    fn garbage_bytes_fall_back() {
        let junk = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let (out, ok) = ensure_rec_argmax(junk.clone());
        assert!(!ok);
        assert_eq!(out, junk);
    }
}
