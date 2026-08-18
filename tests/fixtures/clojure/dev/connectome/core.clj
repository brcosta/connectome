(ns dev.connectome.core)

(defn decorate [name]
  (str "Hi " name))

(defn greet [name]
  (decorate name))
